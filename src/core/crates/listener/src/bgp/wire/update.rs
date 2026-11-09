// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The UPDATE message (RFC 4271 §4.3) with the attributes a route row keeps,
//! read the way RFC 7606 asks for.
//!
//! [`Update::parse`] fails when RFC 7606 calls for a "session reset": the
//! message's structure is broken so its prefixes cannot be known, an
//! MP_REACH_NLRI or MP_UNREACH_NLRI is incorrect (§5.3, §7.11), or a
//! malformed attribute comes with no reachable NLRI (§5.2). The error carries
//! the NOTIFICATION subcode and data (RFC 4271 §6.3). A malformed attribute
//! whose bounds are intact sets [`Attrs::withdraw`] instead, and the caller
//! withdraws whatever the message announces ("treat-as-withdraw"); one that
//! cannot affect a route is dropped ("attribute discard").

use super::bgp::subcode;
use super::open::AS_TRANS;
use super::{AFI_IPV4, AFI_IPV6, Afi, Reader, Result, WireError};
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::ops::Range;

const ATTR_ORIGIN: u8 = 1;
const ATTR_AS_PATH: u8 = 2;
const ATTR_NEXT_HOP: u8 = 3;
const ATTR_MED: u8 = 4;
const ATTR_LOCAL_PREF: u8 = 5;
const ATTR_ATOMIC_AGGREGATE: u8 = 6;
const ATTR_AGGREGATOR: u8 = 7;
const ATTR_COMMUNITIES: u8 = 8;
const ATTR_ORIGINATOR_ID: u8 = 9;
const ATTR_CLUSTER_LIST: u8 = 10;
const ATTR_MP_REACH: u8 = 14;
const ATTR_MP_UNREACH: u8 = 15;
const ATTR_EXT_COMMUNITIES: u8 = 16;
const ATTR_AS4_PATH: u8 = 17;
const ATTR_AS4_AGGREGATOR: u8 = 18;
const ATTR_IPV6_EXT_COMMUNITIES: u8 = 25;
const ATTR_LARGE_COMMUNITIES: u8 = 32;
const FLAG_OPTIONAL: u8 = 0x80;
const FLAG_TRANSITIVE: u8 = 0x40;
const FLAG_EXTENDED_LENGTH: u8 = 0x10;

/// The Optional and Transitive bits an attribute patrolli recognizes must
/// carry (RFC 7606 §3 c); none for the rest.
fn expected_flags(kind: u8) -> Option<u8> {
    const WELL_KNOWN: u8 = FLAG_TRANSITIVE;
    const OPTIONAL: u8 = FLAG_OPTIONAL;
    const OPTIONAL_TRANSITIVE: u8 = FLAG_OPTIONAL | FLAG_TRANSITIVE;
    match kind {
        ATTR_ORIGIN | ATTR_AS_PATH | ATTR_NEXT_HOP | ATTR_LOCAL_PREF | ATTR_ATOMIC_AGGREGATE => {
            Some(WELL_KNOWN)
        }
        ATTR_MED | ATTR_ORIGINATOR_ID | ATTR_CLUSTER_LIST | ATTR_MP_REACH | ATTR_MP_UNREACH => {
            Some(OPTIONAL)
        }
        ATTR_AGGREGATOR
        | ATTR_COMMUNITIES
        | ATTR_EXT_COMMUNITIES
        | ATTR_AS4_PATH
        | ATTR_AS4_AGGREGATOR
        | ATTR_IPV6_EXT_COMMUNITIES
        | ATTR_LARGE_COMMUNITIES => Some(OPTIONAL_TRANSITIVE),
        _ => None,
    }
}

/// What the session that carried an UPDATE says about reading it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Session {
    /// AS_PATH and AGGREGATOR carry 4-octet AS numbers (RFC 6793); AS4_PATH
    /// and AS4_AGGREGATOR are then discarded
    pub asn4: bool,
    /// per unicast family, whether prefixes carry a path id (RFC 7911)
    pub add_path: [bool; 2],
    /// The sender is an external peer: LOCAL_PREF, ORIGINATOR_ID and
    /// CLUSTER_LIST are discarded (RFC 4271 §5.1.5, RFC 7606 §7.5, §7.9,
    /// §7.10) and confederation segments make the AS_PATH malformed
    /// (RFC 5065 §5). BMP feeds are read as internal.
    pub external: bool,
}

/// An UPDATE that ends the session, with the UPDATE Message Error subcode
/// and data of the NOTIFICATION that says why (RFC 4271 §6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateError {
    pub reason: &'static str,
    pub subcode: u8,
    pub data: Vec<u8>,
}

impl UpdateError {
    fn new(reason: &'static str, subcode: u8) -> Self {
        Self {
            reason,
            subcode,
            data: Vec::new(),
        }
    }

    /// An incorrect MP_REACH_NLRI or MP_UNREACH_NLRI (RFC 4760 §7: Optional
    /// Attribute Error, the attribute as data).
    fn mp(e: WireError, attr: &[u8]) -> Self {
        Self {
            reason: e.0,
            subcode: subcode::OPTIONAL_ATTRIBUTE_ERROR,
            data: attr.to_vec(),
        }
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason)
    }
}

impl std::error::Error for UpdateError {}

/// The first malformed attribute of a message withdrawn by RFC 7606's
/// "treat-as-withdraw".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed<'a> {
    pub reason: &'static str,
    /// the UPDATE Message Error subcode, should the message need a session
    /// reset after all (RFC 7606 §5.2)
    pub subcode: u8,
    /// the attribute, flags to value; empty when it is missing or could not
    /// be framed
    pub attr: &'a [u8],
}

impl From<Malformed<'_>> for UpdateError {
    fn from(m: Malformed<'_>) -> Self {
        // RFC 4271 §6.3: these subcodes carry the erroneous attribute
        let data = match m.subcode {
            subcode::ATTRIBUTE_FLAGS_ERROR
            | subcode::ATTRIBUTE_LENGTH_ERROR
            | subcode::INVALID_ORIGIN_ATTRIBUTE
            | subcode::OPTIONAL_ATTRIBUTE_ERROR => m.attr.to_vec(),
            _ => Vec::new(),
        };
        Self {
            reason: m.reason,
            subcode: m.subcode,
            data,
        }
    }
}

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
    /// raw segments, checked as 4-octet: what a 2-octet session's AS_PATH
    /// is merged with (RFC 6793 §4.2.3); none when the session is 4-octet,
    /// when it was malformed, or when the AGGREGATOR rule says to ignore it
    pub as4_path: Option<&'a [u8]>,
    pub next_hop: Option<Ipv4Addr>,
    pub med: Option<u32>,
    /// none from an external peer
    pub local_pref: Option<u32>,
    /// the AS of a well-formed AGGREGATOR
    pub aggregator_as: Option<u32>,
    /// a well-formed AS4_AGGREGATOR came on a 2-octet session
    pub as4_aggregator: bool,
    /// a non-zero multiple of 4 bytes
    pub communities: Option<&'a [u8]>,
    /// a non-zero multiple of 12 bytes
    pub large_communities: Option<&'a [u8]>,
    pub mp_reach: Option<MpReach<'a>>,
    pub mp_unreach: Option<MpUnreach<'a>>,
    /// Set when an attribute the route depends on is malformed or missing:
    /// everything the message announces must be withdrawn instead (RFC 7606
    /// §2).
    pub withdraw: Option<Malformed<'a>>,
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
    /// Reads an UPDATE body (after the BGP header) from an internal sender.
    /// `asn4`: AS_PATH carries 4-octet AS numbers. `add_path`: per unicast
    /// family, whether prefixes carry a path id. See [`Self::read`].
    pub fn parse(
        body: &'a [u8],
        asn4: bool,
        add_path: [bool; 2],
    ) -> std::result::Result<Self, UpdateError> {
        Self::read(
            body,
            Session {
                asn4,
                add_path,
                external: false,
            },
        )
    }

    /// Reads an UPDATE body (after the BGP header). The NLRI are checked
    /// here, so iterating them later cannot fail.
    pub fn read(body: &'a [u8], session: Session) -> std::result::Result<Self, UpdateError> {
        let mut r = Reader::new(body);
        // RFC 4271 §6.3 (kept by RFC 7606 §3 b): lengths past the message
        let lengths = |e: WireError| UpdateError::new(e.0, subcode::MALFORMED_ATTRIBUTE_LIST);
        let withdrawn_len = r.u16("update truncated").map_err(lengths)? as usize;
        let withdrawn = r
            .take(withdrawn_len, "withdrawn routes overrun the update")
            .map_err(lengths)?;
        let attrs_len = r.u16("update truncated").map_err(lengths)? as usize;
        let attrs = r
            .take(attrs_len, "path attributes overrun the update")
            .map_err(lengths)?;
        let nlri = r.rest();

        // RFC 4271 §6.3, RFC 7606 §3 i: both fields, Invalid Network Field
        let v4_add_path = session.add_path[Afi::V4.index()];
        let network = |e: WireError| UpdateError::new(e.0, subcode::INVALID_NETWORK_FIELD);
        check_prefixes(withdrawn, Afi::V4, v4_add_path).map_err(network)?;
        check_prefixes(nlri, Afi::V4, v4_add_path).map_err(network)?;
        let attrs = Attrs::parse(attrs, session, !nlri.is_empty())?;
        Ok(Self {
            withdrawn,
            nlri,
            attrs,
        })
    }

    /// The AFI/SAFI this message marks the End-of-RIB of (RFC 4724 §2): an
    /// empty UPDATE for IPv4 unicast, otherwise an UPDATE whose only
    /// attribute is an MP_UNREACH_NLRI with no prefixes. Anything looser
    /// would sweep a family in the middle of its dump.
    pub fn end_of_rib(&self) -> Option<(u16, u8)> {
        if !self.withdrawn.is_empty() || !self.nlri.is_empty() || self.attrs.mp_reach.is_some() {
            return None;
        }
        match self.attrs.mp_unreach {
            None if self.attrs.count == 0 => Some((AFI_IPV4, super::SAFI_UNICAST)),
            Some(u) if u.nlri.is_empty() && self.attrs.count == 1 => Some((u.afi, u.safi)),
            _ => None,
        }
    }
}

impl<'a> Attrs<'a> {
    /// Reads the path attributes. `v4_nlri`: the message's NLRI field is not
    /// empty.
    fn parse(bytes: &'a [u8], s: Session, v4_nlri: bool) -> std::result::Result<Self, UpdateError> {
        let mut r = Reader::new(bytes);
        let mut attrs = Attrs::default();
        let mut seen = [false; 256];
        while !r.is_empty() {
            let start = r;
            let Ok((flags, kind, value)) = attribute(&mut r) else {
                // RFC 7606 §4: an attribute header or value past the field
                // withdraws the routes; the Total Attribute Length still
                // frames the NLRI field
                attrs.malformed(
                    "attribute overruns the attributes",
                    subcode::MALFORMED_ATTRIBUTE_LIST,
                    &[],
                );
                break;
            };
            let raw = &start.rest()[..start.len() - r.len()];
            attrs.count += 1;
            let first = !std::mem::replace(&mut seen[kind as usize], true);
            let mp = matches!(kind, ATTR_MP_REACH | ATTR_MP_UNREACH);
            if mp && !first {
                // RFC 7606 §3 g
                return Err(UpdateError::new(
                    "mp_reach or mp_unreach repeated",
                    subcode::MALFORMED_ATTRIBUTE_LIST,
                ));
            }
            if !first {
                // RFC 7606 §3 g: later copies are discarded
                continue;
            }
            // discarded before anything is checked: RFC 4271 §5.1.5, RFC 7606
            // §7.5, §7.9, §7.10 for an external peer; RFC 6793 §6 for the
            // AS4 attributes between 4-octet speakers
            let discard = match kind {
                ATTR_LOCAL_PREF | ATTR_ORIGINATOR_ID | ATTR_CLUSTER_LIST => s.external,
                ATTR_AS4_PATH | ATTR_AS4_AGGREGATOR => s.asn4,
                _ => false,
            };
            if discard {
                continue;
            }
            if let Some(expected) = expected_flags(kind)
                && flags & (FLAG_OPTIONAL | FLAG_TRANSITIVE) != expected
            {
                match kind {
                    // RFC 7606 §5.3: an incorrect MP attribute
                    _ if mp => {
                        return Err(UpdateError::mp(WireError("mp attribute flags"), raw));
                    }
                    // attribute discard: RFC 7606 §3 f, RFC 6793 §6
                    ATTR_ATOMIC_AGGREGATE
                    | ATTR_AGGREGATOR
                    | ATTR_AS4_PATH
                    | ATTR_AS4_AGGREGATOR => {}
                    // RFC 7606 §3 c
                    _ => attrs.malformed("attribute flags", subcode::ATTRIBUTE_FLAGS_ERROR, raw),
                }
                continue;
            }
            attrs.read(kind, value, raw, s)?;
        }

        // RFC 6793 §4.2.3: an AGGREGATOR that is not AS_TRANS beside an
        // AS4_AGGREGATOR means the AS_PATH is the whole path
        if attrs.as4_aggregator
            && attrs
                .aggregator_as
                .is_some_and(|asn| asn != u32::from(AS_TRANS))
        {
            attrs.as4_path = None;
        }

        let reach = v4_nlri || attrs.mp_reach.is_some_and(|m| !m.nlri.is_empty());
        if reach {
            // RFC 7606 §3 d; RFC 4760 §3 makes NEXT_HOP needed only beside
            // IPv4 NLRI
            let missing = [
                (ATTR_ORIGIN, "missing origin", true),
                (ATTR_AS_PATH, "missing as_path", true),
                (ATTR_NEXT_HOP, "missing next_hop", v4_nlri),
            ];
            for (kind, reason, needed) in missing {
                if needed && !seen[kind as usize] {
                    attrs.malformed(reason, subcode::MISSING_WELL_KNOWN_ATTRIBUTE, &[]);
                }
            }
        } else if let Some(m) = attrs.withdraw {
            // RFC 7606 §5.2: with no reachable NLRI, nothing shows the
            // message was framed right
            return Err(m.into());
        }
        Ok(attrs)
    }

    /// Checks one attribute and keeps what a route row needs.
    fn read(
        &mut self,
        kind: u8,
        value: &'a [u8],
        raw: &'a [u8],
        s: Session,
    ) -> std::result::Result<(), UpdateError> {
        let u32_of = |v: &[u8]| <[u8; 4]>::try_from(v).ok().map(u32::from_be_bytes);
        // a non-zero multiple of `n` bytes (RFC 7606 §7.8, §7.10, §7.14,
        // §7.15, RFC 8092 §6)
        let multiple = |v: &[u8], n: usize| !v.is_empty() && v.len().is_multiple_of(n);
        match kind {
            ATTR_ORIGIN => match value {
                [0..=2] => {}
                [_] => self.malformed("malformed origin", subcode::INVALID_ORIGIN_ATTRIBUTE, raw),
                _ => self.malformed("malformed origin", subcode::ATTRIBUTE_LENGTH_ERROR, raw),
            },
            ATTR_AS_PATH => match AsPath::check(value, s.asn4) {
                // RFC 5065 §5: confederation segments only within one
                Ok(confed) if confed && s.external => {
                    self.malformed(
                        "as_path confederation segment from an external peer",
                        subcode::MALFORMED_AS_PATH,
                        raw,
                    );
                }
                Ok(_) => self.as_path = Some(value),
                Err(_) => self.malformed("malformed as_path", subcode::MALFORMED_AS_PATH, raw),
            },
            ATTR_AS4_PATH => {
                // RFC 6793 §6: at least one AS, else malformed; a malformed
                // AS4_PATH is discarded
                if value.len() >= 6
                    && value.len().is_multiple_of(2)
                    && AsPath::check(value, true).is_ok()
                {
                    self.as4_path = Some(value);
                }
            }
            ATTR_NEXT_HOP => match <[u8; 4]>::try_from(value) {
                Ok(ip) => self.next_hop = Some(Ipv4Addr::from(ip)),
                Err(_) => {
                    self.malformed("malformed next_hop", subcode::ATTRIBUTE_LENGTH_ERROR, raw)
                }
            },
            ATTR_MED => match u32_of(value) {
                Some(v) => self.med = Some(v),
                None => self.malformed("malformed med", subcode::ATTRIBUTE_LENGTH_ERROR, raw),
            },
            ATTR_LOCAL_PREF => match u32_of(value) {
                Some(v) => self.local_pref = Some(v),
                None => {
                    self.malformed("malformed local_pref", subcode::ATTRIBUTE_LENGTH_ERROR, raw);
                }
            },
            // RFC 7606 §7.7, RFC 7607 §2: a malformed one is discarded
            ATTR_AGGREGATOR => {
                let asn = match (s.asn4, value.len()) {
                    (true, 8) => u32_of(&value[..4]),
                    (false, 6) => Some(u16::from_be_bytes([value[0], value[1]]).into()),
                    _ => None,
                };
                self.aggregator_as = asn.filter(|asn| *asn != 0);
            }
            // RFC 6793 §6, RFC 7607 §2: likewise
            ATTR_AS4_AGGREGATOR => {
                self.as4_aggregator = value.len() == 8 && u32_of(&value[..4]) != Some(0);
            }
            ATTR_COMMUNITIES => {
                if multiple(value, 4) {
                    self.communities = Some(value);
                } else {
                    self.malformed(
                        "malformed communities",
                        subcode::OPTIONAL_ATTRIBUTE_ERROR,
                        raw,
                    );
                }
            }
            ATTR_LARGE_COMMUNITIES => {
                if multiple(value, 12) {
                    self.large_communities = Some(value);
                } else {
                    self.malformed(
                        "malformed large_communities",
                        subcode::OPTIONAL_ATTRIBUTE_ERROR,
                        raw,
                    );
                }
            }
            // read only to be checked; from an external peer they were
            // discarded already
            ATTR_ORIGINATOR_ID if value.len() != 4 => {
                self.malformed(
                    "malformed originator_id",
                    subcode::OPTIONAL_ATTRIBUTE_ERROR,
                    raw,
                );
            }
            ATTR_CLUSTER_LIST if !multiple(value, 4) => {
                self.malformed(
                    "malformed cluster_list",
                    subcode::OPTIONAL_ATTRIBUTE_ERROR,
                    raw,
                );
            }
            ATTR_EXT_COMMUNITIES if !multiple(value, 8) => {
                self.malformed(
                    "malformed extended_communities",
                    subcode::OPTIONAL_ATTRIBUTE_ERROR,
                    raw,
                );
            }
            ATTR_IPV6_EXT_COMMUNITIES if !multiple(value, 20) => {
                self.malformed(
                    "malformed ipv6_extended_communities",
                    subcode::OPTIONAL_ATTRIBUTE_ERROR,
                    raw,
                );
            }
            ATTR_MP_REACH => {
                let reach =
                    MpReach::parse(value, s.add_path).map_err(|e| UpdateError::mp(e, raw))?;
                self.mp_reach = Some(reach);
            }
            ATTR_MP_UNREACH => {
                let unreach =
                    MpUnreach::parse(value, s.add_path).map_err(|e| UpdateError::mp(e, raw))?;
                self.mp_unreach = Some(unreach);
            }
            // ATOMIC_AGGREGATE (RFC 7606 §7.6: a malformed one is discarded,
            // and nothing reads it) and everything unknown
            _ => {}
        }
        Ok(())
    }

    fn malformed(&mut self, reason: &'static str, subcode: u8, attr: &'a [u8]) {
        self.withdraw.get_or_insert(Malformed {
            reason,
            subcode,
            attr,
        });
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

/// One attribute's flags, type code and value.
fn attribute<'a>(r: &mut Reader<'a>) -> Result<(u8, u8, &'a [u8])> {
    let flags = r.u8("attribute header truncated")?;
    let kind = r.u8("attribute header truncated")?;
    let len = if flags & FLAG_EXTENDED_LENGTH != 0 {
        r.u16("attribute header truncated")? as usize
    } else {
        r.u8("attribute header truncated")? as usize
    };
    let value = r.take(len, "attribute overruns the attributes")?;
    Ok((flags, kind, value))
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
                Some(next_hop(afi, nh)?)
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

/// An MP_REACH_NLRI next hop of a unicast `afi`: 16 bytes (IPv6 global) or
/// 32 (global plus link-local, RFC 2545 §3; the link-local one used only
/// when the global one is unspecified); for IPv4 routes also 4 (IPv4), and
/// 16 or 32 per RFC 8950. Any other length is inconsistent (RFC 7606 §7.11).
fn next_hop(afi: u16, nh: &[u8]) -> Result<IpAddr> {
    match nh.len() {
        4 if afi == AFI_IPV4 => Ok(IpAddr::V4(Ipv4Addr::from(
            <[u8; 4]>::try_from(nh).unwrap_or_default(),
        ))),
        16 | 32 if matches!(afi, AFI_IPV4 | AFI_IPV6) => {
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
    /// known types, no empty segment, nothing left over, and no AS 0
    /// (RFC 7607 §2). Returns whether a confederation segment is among them.
    pub fn check(bytes: &[u8], asn4: bool) -> Result<bool> {
        let width = if asn4 { 4 } else { 2 };
        let mut r = Reader::new(bytes);
        let mut confed = false;
        while !r.is_empty() {
            let kind = SegmentKind::from_wire(r.u8("as path segment truncated")?)
                .ok_or(WireError("unknown as path segment type"))?;
            confed |= kind.is_confed();
            let count = r.u8("as path segment truncated")? as usize;
            if count == 0 {
                return Err(WireError("empty as path segment"));
            }
            let asns = r.take(count * width, "as path segment truncated")?;
            if asns
                .chunks_exact(width)
                .any(|asn| asn.iter().all(|b| *b == 0))
            {
                return Err(WireError("as number zero"));
            }
        }
        Ok(confed)
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
    /// cover, then `as4` without confederation segments (RFC 6793 §6). A
    /// confederation segment of `self` goes along when it leads the path or
    /// is next to a segment that goes along. `as4` is ignored when it is the
    /// longer of the two.
    pub fn merge_as4(&mut self, as4: &AsPath) {
        let (n, m) = (self.counted_len(), as4.counted_len());
        if m > n {
            return;
        }
        let mut keep = n - m;
        let segments: Vec<(SegmentKind, &[u32])> = self.segments().collect();
        // how many ASes of each segment are prepended
        let mut take = vec![0; segments.len()];
        for (i, (kind, asns)) in segments.iter().enumerate() {
            match kind {
                SegmentKind::Sequence => {
                    take[i] = asns.len().min(keep);
                    keep -= take[i];
                }
                SegmentKind::Set if keep > 0 => {
                    take[i] = asns.len();
                    keep -= 1;
                }
                _ => {}
            }
        }
        // confederation segments, until none is added: one next to another
        // that was added goes too
        loop {
            let mut added = false;
            for i in 0..segments.len() {
                let (kind, asns) = segments[i];
                if !kind.is_confed() || take[i] > 0 {
                    continue;
                }
                let next_to_taken =
                    (i > 0 && take[i - 1] > 0) || take.get(i + 1).is_some_and(|t| *t > 0);
                if i == 0 || next_to_taken {
                    take[i] = asns.len();
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
        let mut merged = AsPath::default();
        for ((kind, asns), take) in segments.iter().zip(take) {
            if take > 0 {
                merged.push(*kind, &asns[..take]);
            }
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
    use crate::bgp::wire::{AFI_IPV6, SAFI_UNICAST};

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
        // nor an empty MP_UNREACH with other attributes beside it, either
        // order
        for attrs in [
            [origin(0), mp_unreach(AFI_IPV6, SAFI_UNICAST, &[])],
            [mp_unreach(AFI_IPV6, SAFI_UNICAST, &[]), origin(0)],
        ] {
            let body = update(&[], &attrs, &[]);
            let u = Update::parse(&body, true, [false; 2]).unwrap();
            assert_eq!(u.end_of_rib(), None);
        }
    }

    /// ORIGIN, AS_PATH and NEXT_HOP: what an IPv4 route needs.
    fn mandatory() -> Vec<Vec<u8>> {
        vec![
            origin(0),
            as_path(&[(2, &[64500])]),
            next_hop_attr("192.0.2.1"),
        ]
    }

    /// An IPv4 route with the mandatory attributes, any of the same type
    /// replaced by `extra`, read in `session`.
    fn route_with(
        extra: &[Vec<u8>],
        session: Session,
    ) -> std::result::Result<Update<'static>, UpdateError> {
        let mut attrs: Vec<Vec<u8>> = mandatory()
            .into_iter()
            .filter(|a| extra.iter().all(|e| e[1] != a[1]))
            .collect();
        attrs.extend_from_slice(extra);
        let body = update(&[], &attrs, &nlri(&["10.0.0.0/8"], None)).leak();
        Update::read(body, session)
    }

    const INTERNAL: Session = Session {
        asn4: true,
        add_path: [false; 2],
        external: false,
    };
    const EXTERNAL: Session = Session {
        external: true,
        ..INTERNAL
    };

    /// (reason, subcode) of what `extra` withdraws.
    fn withdrawn_by(extra: &[Vec<u8>], session: Session) -> Option<(&'static str, u8)> {
        let u = route_with(extra, session).unwrap();
        assert_eq!(nets(u.nlri, Afi::V4, false).len(), 1, "the NLRI still read");
        u.attrs.withdraw.map(|m| (m.reason, m.subcode))
    }

    #[test]
    fn a_malformed_route_attribute_means_withdraw() {
        use subcode::*;
        let ot = OPTIONAL | TRANSITIVE;
        for (bad, want) in [
            (
                attr(WELL_KNOWN, ATTR_ORIGIN, &[3]),
                ("malformed origin", INVALID_ORIGIN_ATTRIBUTE),
            ),
            (
                attr(WELL_KNOWN, ATTR_ORIGIN, &[0, 0]),
                ("malformed origin", ATTRIBUTE_LENGTH_ERROR),
            ),
            (
                attr(WELL_KNOWN, ATTR_AS_PATH, &[2, 0]),
                ("malformed as_path", MALFORMED_AS_PATH),
            ),
            (
                attr(WELL_KNOWN, ATTR_AS_PATH, &[9, 1, 0, 0, 0, 1]),
                ("malformed as_path", MALFORMED_AS_PATH),
            ),
            (
                attr(WELL_KNOWN, ATTR_AS_PATH, &[2, 2, 0, 0, 0, 1]),
                ("malformed as_path", MALFORMED_AS_PATH),
            ),
            // RFC 7607 §2: AS 0
            (
                as_path(&[(2, &[64500, 0])]),
                ("malformed as_path", MALFORMED_AS_PATH),
            ),
            (
                attr(WELL_KNOWN, ATTR_NEXT_HOP, &[1, 2, 3]),
                ("malformed next_hop", ATTRIBUTE_LENGTH_ERROR),
            ),
            (
                attr(OPTIONAL, ATTR_MED, &[1]),
                ("malformed med", ATTRIBUTE_LENGTH_ERROR),
            ),
            (
                attr(WELL_KNOWN, ATTR_LOCAL_PREF, &[]),
                ("malformed local_pref", ATTRIBUTE_LENGTH_ERROR),
            ),
            (
                attr(ot, ATTR_COMMUNITIES, &[1, 2, 3]),
                ("malformed communities", OPTIONAL_ATTRIBUTE_ERROR),
            ),
            (
                attr(ot, ATTR_COMMUNITIES, &[]),
                ("malformed communities", OPTIONAL_ATTRIBUTE_ERROR),
            ),
            (
                attr(ot, ATTR_LARGE_COMMUNITIES, &[0; 8]),
                ("malformed large_communities", OPTIONAL_ATTRIBUTE_ERROR),
            ),
            // RFC 7606 §7.9, §7.10, §7.14, §7.15
            (
                attr(OPTIONAL, ATTR_ORIGINATOR_ID, &[1, 2, 3]),
                ("malformed originator_id", OPTIONAL_ATTRIBUTE_ERROR),
            ),
            (
                attr(OPTIONAL, ATTR_CLUSTER_LIST, &[]),
                ("malformed cluster_list", OPTIONAL_ATTRIBUTE_ERROR),
            ),
            (
                attr(OPTIONAL, ATTR_CLUSTER_LIST, &[0; 6]),
                ("malformed cluster_list", OPTIONAL_ATTRIBUTE_ERROR),
            ),
            (
                attr(ot, ATTR_EXT_COMMUNITIES, &[0; 12]),
                ("malformed extended_communities", OPTIONAL_ATTRIBUTE_ERROR),
            ),
            (
                attr(ot, ATTR_EXT_COMMUNITIES, &[]),
                ("malformed extended_communities", OPTIONAL_ATTRIBUTE_ERROR),
            ),
            (
                attr(ot, ATTR_IPV6_EXT_COMMUNITIES, &[0; 8]),
                (
                    "malformed ipv6_extended_communities",
                    OPTIONAL_ATTRIBUTE_ERROR,
                ),
            ),
            // RFC 7606 §3 c: the Optional and Transitive bits
            (
                attr(OPTIONAL, ATTR_ORIGIN, &[0]),
                ("attribute flags", ATTRIBUTE_FLAGS_ERROR),
            ),
            (
                attr(WELL_KNOWN, ATTR_MED, &[0; 4]),
                ("attribute flags", ATTRIBUTE_FLAGS_ERROR),
            ),
            (
                attr(OPTIONAL, ATTR_COMMUNITIES, &[0; 4]),
                ("attribute flags", ATTRIBUTE_FLAGS_ERROR),
            ),
            (
                attr(ot, ATTR_LOCAL_PREF, &[0; 4]),
                ("attribute flags", ATTRIBUTE_FLAGS_ERROR),
            ),
        ] {
            assert_eq!(
                withdrawn_by(std::slice::from_ref(&bad), INTERNAL),
                Some(want),
                "{bad:?}"
            );
        }
        // well-formed ones, extended length and the Partial bit included
        let fine = [
            attr(OPTIONAL, ATTR_ORIGINATOR_ID, &[1, 2, 3, 4]),
            attr(OPTIONAL, ATTR_CLUSTER_LIST, &[0; 8]),
            attr(ot, ATTR_EXT_COMMUNITIES, &[0; 16]),
            attr(ot | 0x20, ATTR_IPV6_EXT_COMMUNITIES, &[0; 20]),
            attr(WELL_KNOWN, ATTR_ATOMIC_AGGREGATE, &[]),
            communities(&[1; 100]),
        ];
        assert_eq!(withdrawn_by(&fine, INTERNAL), None);
    }

    #[test]
    fn attributes_that_cannot_change_a_route_are_discarded() {
        let ot = OPTIONAL | TRANSITIVE;
        // RFC 7606 §3 f, §7.6, §7.7, RFC 6793 §6: whatever is wrong with them
        for discarded in [
            attr(WELL_KNOWN, ATTR_ATOMIC_AGGREGATE, &[1]),
            attr(OPTIONAL, ATTR_ATOMIC_AGGREGATE, &[]),
            attr(ot, ATTR_AGGREGATOR, &[0; 3]),
            attr(WELL_KNOWN, ATTR_AGGREGATOR, &[0, 0, 0, 1, 1, 2, 3, 4]),
            attr(
                WELL_KNOWN,
                ATTR_AS4_PATH,
                &segments_bytes(&[(2, &[1])], true),
            ),
            attr(ot, ATTR_AS4_AGGREGATOR, &[0; 5]),
        ] {
            let session = Session {
                asn4: false,
                ..INTERNAL
            };
            let path = as_path2(&[(2, &[64500])]);
            let u = route_with(&[discarded.clone(), path], session).unwrap();
            assert_eq!(u.attrs.withdraw, None, "{discarded:?}");
            assert_eq!(u.attrs.aggregator_as, None);
            assert!(!u.attrs.as4_aggregator);
        }
    }

    #[test]
    fn an_external_peer_has_its_internal_attributes_discarded() {
        let lp = local_pref(200);
        let originator = attr(OPTIONAL, ATTR_ORIGINATOR_ID, &[1, 2, 3, 4]);
        // RFC 4271 §5.1.5: from an internal peer kept, from an external one
        // ignored
        assert_eq!(
            route_with(std::slice::from_ref(&lp), INTERNAL)
                .unwrap()
                .attrs
                .local_pref,
            Some(200)
        );
        assert_eq!(
            route_with(&[lp, originator], EXTERNAL)
                .unwrap()
                .attrs
                .local_pref,
            None
        );
        // RFC 7606 §7.5, §7.9, §7.10: malformed ones are discarded, not a
        // withdrawal
        let bad = [
            attr(WELL_KNOWN, ATTR_LOCAL_PREF, &[1]),
            attr(OPTIONAL, ATTR_ORIGINATOR_ID, &[1]),
            attr(OPTIONAL, ATTR_CLUSTER_LIST, &[1]),
        ];
        assert_eq!(withdrawn_by(&bad, EXTERNAL), None);
        assert!(withdrawn_by(&bad, INTERNAL).is_some());
    }

    #[test]
    fn confederation_segments_from_an_external_peer_are_malformed() {
        // RFC 5065 §5
        let confed = as_path(&[(3, &[65001]), (2, &[64500])]);
        assert_eq!(withdrawn_by(std::slice::from_ref(&confed), INTERNAL), None);
        assert_eq!(
            withdrawn_by(&[confed], EXTERNAL),
            Some((
                "as_path confederation segment from an external peer",
                subcode::MALFORMED_AS_PATH
            ))
        );
    }

    #[test]
    fn a_missing_well_known_attribute_means_withdraw() {
        // RFC 7606 §3 d
        for (drop, reason) in [
            (ATTR_ORIGIN, "missing origin"),
            (ATTR_AS_PATH, "missing as_path"),
            (ATTR_NEXT_HOP, "missing next_hop"),
        ] {
            let attrs: Vec<Vec<u8>> = mandatory().into_iter().filter(|a| a[1] != drop).collect();
            let body = update(&[], &attrs, &nlri(&["10.0.0.0/8"], None));
            let u = Update::parse(&body, true, [false; 2]).unwrap();
            assert_eq!(
                u.attrs.withdraw.map(|m| (m.reason, m.subcode)),
                Some((reason, subcode::MISSING_WELL_KNOWN_ATTRIBUTE))
            );
        }
        // RFC 4760 §3: NEXT_HOP is needed only beside IPv4 NLRI
        let nh = [0x20, 1, 0xd, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let reach = mp_reach(AFI_IPV6, SAFI_UNICAST, &nh, &nlri(&["2001:db8::/32"], None));
        let body = update(&[], &[origin(0), as_path(&[]), reach.clone()], &[]);
        assert_eq!(
            Update::parse(&body, true, [false; 2])
                .unwrap()
                .attrs
                .withdraw,
            None
        );
        let body = update(&[], &[as_path(&[]), reach], &[]);
        let u = Update::parse(&body, true, [false; 2]).unwrap();
        assert_eq!(u.attrs.withdraw.map(|m| m.reason), Some("missing origin"));
        // withdrawals need none
        let body = update(&nlri(&["10.0.0.0/8"], None), &[], &[]);
        assert_eq!(
            Update::parse(&body, true, [false; 2])
                .unwrap()
                .attrs
                .withdraw,
            None
        );
    }

    #[test]
    fn a_malformed_attribute_without_reachable_nlri_resets_the_session() {
        // RFC 7606 §5.2: withdrawals beside a bad MED
        let bad_med = attr(OPTIONAL, ATTR_MED, &[1]);
        let body = update(
            &nlri(&["10.0.0.0/8"], None),
            std::slice::from_ref(&bad_med),
            &[],
        );
        let e = Update::parse(&body, true, [false; 2]).unwrap_err();
        assert_eq!(e.reason, "malformed med");
        assert_eq!(e.subcode, subcode::ATTRIBUTE_LENGTH_ERROR);
        assert_eq!(e.data, bad_med, "RFC 4271 §6.3: the erroneous attribute");
        // an MP_REACH with no NLRI is no reachable NLRI either
        let reach = mp_reach(AFI_IPV6, SAFI_UNICAST, &[0; 16], &[]);
        let body = update(&[], &[origin(9), reach], &[]);
        let e = Update::parse(&body, true, [false; 2]).unwrap_err();
        assert_eq!(e.subcode, subcode::INVALID_ORIGIN_ATTRIBUTE);
        // AS_PATH errors carry no data
        let body = update(&[], &[attr(WELL_KNOWN, ATTR_AS_PATH, &[2, 0])], &[]);
        let e = Update::parse(&body, true, [false; 2]).unwrap_err();
        assert_eq!((e.subcode, e.data.len()), (subcode::MALFORMED_AS_PATH, 0));
        // a discarded attribute is no reason
        let body = update(&[], &[attr(WELL_KNOWN, ATTR_ATOMIC_AGGREGATE, &[1])], &[]);
        assert!(Update::parse(&body, true, [false; 2]).is_ok());
    }

    #[test]
    fn a_truncated_attribute_header_means_withdraw() {
        // RFC 7606 §4: fewer bytes than an attribute header after the last
        let mut attrs = mandatory();
        attrs.push(vec![0x40, 1]);
        let body = update(&[], &attrs, &nlri(&["10.0.0.0/8"], None));
        let u = Update::parse(&body, true, [false; 2]).unwrap();
        assert_eq!(
            u.attrs.withdraw.map(|m| (m.reason, m.subcode)),
            Some((
                "attribute overruns the attributes",
                subcode::MALFORMED_ATTRIBUTE_LIST
            ))
        );
        assert_eq!(nets(u.nlri, Afi::V4, false).len(), 1);
    }

    #[test]
    fn an_incorrect_mp_attribute_resets_with_an_optional_attribute_error() {
        let route = nlri(&["2001:db8::/32"], None);
        for (reach, reason) in [
            // RFC 2545 §3, RFC 7606 §7.11: an IPv6 next hop is 16 or 32 bytes
            (
                mp_reach(AFI_IPV6, SAFI_UNICAST, &[10, 0, 0, 1], &route),
                "unsupported next hop length",
            ),
            (
                mp_reach(AFI_IPV6, SAFI_UNICAST, &[0; 20], &route),
                "unsupported next hop length",
            ),
            // RFC 7606 §5.3: its flags
            (
                attr(
                    OPTIONAL | TRANSITIVE,
                    ATTR_MP_REACH,
                    &mp_reach(AFI_IPV6, SAFI_UNICAST, &[0; 16], &route)[3..],
                ),
                "mp attribute flags",
            ),
        ] {
            let body = update(&[], &[origin(0), as_path(&[]), reach.clone()], &[]);
            let e = Update::parse(&body, true, [false; 2]).unwrap_err();
            assert_eq!(
                (e.reason, e.subcode),
                (reason, subcode::OPTIONAL_ATTRIBUTE_ERROR)
            );
            assert_eq!(e.data, reach, "RFC 4271 §6.3: the attribute");
        }
        // RFC 8950: an IPv4 route may have a 16 or 32 byte next hop
        for len in [4, 16, 32] {
            let reach = mp_reach(
                AFI_IPV4,
                SAFI_UNICAST,
                &vec![1; len],
                &nlri(&["10.0.0.0/8"], None),
            );
            let body = update(&[], &[origin(0), as_path(&[]), reach], &[]);
            assert!(Update::parse(&body, true, [false; 2]).is_ok(), "{len}");
        }
    }

    #[test]
    fn a_broken_network_field_is_an_invalid_network_field() {
        // RFC 4271 §6.3, RFC 7606 §3 i: the NLRI and the withdrawn routes
        let mut attrs = mandatory();
        let body = update(&[], &attrs, &[33, 10, 0, 0, 0, 0]);
        let e = Update::parse(&body, true, [false; 2]).unwrap_err();
        assert_eq!(e.subcode, subcode::INVALID_NETWORK_FIELD);
        attrs.clear();
        let body = update(&[24, 10, 0], &attrs, &[]);
        let e = Update::parse(&body, true, [false; 2]).unwrap_err();
        assert_eq!(e.subcode, subcode::INVALID_NETWORK_FIELD);
        // lengths past the message: Malformed Attribute List (RFC 7606 §3 b)
        let mut body = update(&[], &mandatory(), &[]);
        body[2..4].copy_from_slice(&999u16.to_be_bytes());
        let e = Update::parse(&body, true, [false; 2]).unwrap_err();
        assert_eq!(e.subcode, subcode::MALFORMED_ATTRIBUTE_LIST);
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
        // an attribute runs past the attributes: RFC 7606 §4 withdraws the
        // routes, the NLRI framed by the Total Attribute Length
        let mut bad = good.clone();
        let first_attr_len = 2 + 2 + 2 + 2;
        bad[first_attr_len] = 200;
        let u = Update::parse(&bad, true, [false; 2]).unwrap();
        assert_eq!(
            u.attrs.withdraw.map(|m| m.reason),
            Some("attribute overruns the attributes")
        );
        assert_eq!(nets(u.nlri, Afi::V4, false).len(), 1);
    }

    #[test]
    fn as4_path_follows_rfc_6793() {
        let two = Session {
            asn4: false,
            ..INTERNAL
        };
        let path = as_path2(&[(2, &[64500, 23456])]);
        let as4 = as4_path(&[(2, &[4_200_000_000])]);
        let aggregator = |asn: u16| {
            let mut v = asn.to_be_bytes().to_vec();
            v.extend_from_slice(&[192, 0, 2, 1]);
            attr(OPTIONAL | TRANSITIVE, ATTR_AGGREGATOR, &v)
        };
        let as4_aggregator = |asn: u32| {
            let mut v = asn.to_be_bytes().to_vec();
            v.extend_from_slice(&[192, 0, 2, 1]);
            attr(OPTIONAL | TRANSITIVE, ATTR_AS4_AGGREGATOR, &v)
        };
        let kept = |extra: &[Vec<u8>]| {
            let mut attrs = vec![path.clone(), as4.clone()];
            attrs.extend_from_slice(extra);
            let u = route_with(&attrs, two).unwrap();
            assert_eq!(u.attrs.withdraw, None);
            u.attrs.as4_path.is_some()
        };
        assert!(kept(&[]));
        // §4.2.3: both aggregators, AGGREGATOR not AS_TRANS: AS_PATH alone
        assert!(!kept(&[aggregator(64500), as4_aggregator(4_200_000_000)]));
        assert!(kept(&[aggregator(23456), as4_aggregator(4_200_000_000)]));
        // only one of them: the rule does not apply
        assert!(kept(&[aggregator(64500)]));
        assert!(kept(&[as4_aggregator(4_200_000_000)]));
        // RFC 7607 §2: an aggregator of AS 0 is malformed, so discarded
        assert!(kept(&[aggregator(0), as4_aggregator(4_200_000_000)]));
        assert!(kept(&[aggregator(64500), as4_aggregator(0)]));

        // §6: shorter than one AS, or with AS 0, is malformed and discarded
        for bad in [
            attr(OPTIONAL | TRANSITIVE, ATTR_AS4_PATH, &[]),
            attr(OPTIONAL | TRANSITIVE, ATTR_AS4_PATH, &[2, 1, 0, 0]),
            as4_path(&[(2, &[0])]),
        ] {
            let u = route_with(&[path.clone(), bad], two).unwrap();
            assert_eq!((u.attrs.withdraw, u.attrs.as4_path), (None, None));
        }
        // between 4-octet speakers it is discarded too
        let u = route_with(&[as_path(&[(2, &[64500])]), as4.clone()], INTERNAL).unwrap();
        assert_eq!(u.attrs.as4_path, None);
    }

    #[test]
    fn only_leading_or_adjacent_confederation_segments_survive_a_merge() {
        // RFC 6793 §4.2.3: two ASes kept from AS_PATH, then AS4_PATH
        let mut path = AsPath::default();
        path.decode(
            &segments_bytes(
                &[
                    (3, &[65001]),
                    (2, &[64500, 64501]),
                    (3, &[65002]),
                    (2, &[23456]),
                    (4, &[65003]),
                    (2, &[23456]),
                ],
                false,
            ),
            false,
        )
        .unwrap();
        let mut as4 = AsPath::default();
        as4.decode(
            &segments_bytes(&[(2, &[4_200_000_000, 4_200_000_001])], true),
            true,
        )
        .unwrap();
        path.merge_as4(&as4);
        // the leading one, and the one next to the sequence kept; not the
        // set between the two AS_TRANS sequences that were dropped
        assert_eq!(
            path_text(&path),
            "ConfedSequence[65001] Sequence[64500, 64501] ConfedSequence[65002] \
             Sequence[4200000000, 4200000001]"
        );
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
