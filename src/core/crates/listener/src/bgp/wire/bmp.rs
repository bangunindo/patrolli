// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! BMP v3 messages (RFC 7854), with the Loc-RIB peer type of RFC 9069.

use super::{Afi, BGP_OPEN, Reader, Result, WireError, bgp_message};
use std::fmt::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const VERSION: u8 = 3;
/// version, length, type
pub const COMMON_HEADER_LEN: usize = 6;
pub const PEER_HEADER_LEN: usize = 42;

pub const ROUTE_MONITORING: u8 = 0;
pub const STATISTICS_REPORT: u8 = 1;
pub const PEER_DOWN: u8 = 2;
pub const PEER_UP: u8 = 3;
pub const INITIATION: u8 = 4;
pub const TERMINATION: u8 = 5;
pub const ROUTE_MIRRORING: u8 = 6;

/// Peer types (RFC 7854 §4.2, RFC 9069 §4.1).
pub const PEER_GLOBAL: u8 = 0;
pub const PEER_RD: u8 = 1;
pub const PEER_LOCAL: u8 = 2;
pub const PEER_LOC_RIB: u8 = 3;

const FLAG_V6: u8 = 0x80;
const FLAG_AS2: u8 = 0x20;

/// Statistics types (RFC 7854 §4.8) with Loc-RIB route counts.
const STAT_LOC_RIB: u16 = 8;
const STAT_LOC_RIB_PER_AFI: u16 = 10;

/// The common header: the message is `length` bytes, this header included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommonHeader {
    pub version: u8,
    pub length: u32,
    pub kind: u8,
}

impl CommonHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        Ok(Self {
            version: r.u8("bmp header truncated")?,
            length: r.u32("bmp header truncated")?,
            kind: r.u8("bmp header truncated")?,
        })
    }
}

/// The per-peer header (RFC 7854 §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerHeader {
    pub peer_type: u8,
    pub flags: u8,
    /// route distinguisher or locally defined instance id; all zero for the
    /// global instance
    pub distinguisher: [u8; 8],
    /// the neighbour's address; unspecified for a Loc-RIB peer
    pub address: IpAddr,
    pub asn: u32,
    pub bgp_id: Ipv4Addr,
}

impl PeerHeader {
    fn parse(r: &mut Reader<'_>) -> Result<Self> {
        let mut h = r.sub(PEER_HEADER_LEN, "peer header truncated")?;
        let peer_type = h.u8("peer header truncated")?;
        let flags = h.u8("peer header truncated")?;
        let distinguisher = h.array("peer header truncated")?;
        let address: [u8; 16] = h.array("peer header truncated")?;
        let asn = h.u32("peer header truncated")?;
        let bgp_id = Ipv4Addr::from(h.u32("peer header truncated")?);
        // the timestamp is not used
        let address = if peer_type != PEER_LOC_RIB && flags & FLAG_V6 == 0 {
            let mut v4 = [0; 4];
            v4.copy_from_slice(address.get(12..).unwrap_or(&[0; 4]));
            IpAddr::V4(Ipv4Addr::from(v4))
        } else {
            IpAddr::V6(Ipv6Addr::from(address))
        };
        Ok(Self {
            peer_type,
            flags,
            distinguisher,
            address,
            asn,
            bgp_id,
        })
    }

    pub fn is_loc_rib(&self) -> bool {
        self.peer_type == PEER_LOC_RIB
    }

    /// Whether AS_PATH carries 4-octet AS numbers: always for Loc-RIB
    /// (RFC 9069 §4.2), otherwise unless the A flag says 2-octet.
    pub fn asn4(&self) -> bool {
        self.is_loc_rib() || self.flags & FLAG_AS2 == 0
    }
}

/// The messages read; route mirroring and unknown types are only counted.
#[derive(Debug, Clone, Copy)]
pub enum Message<'a> {
    RouteMonitoring {
        peer: PeerHeader,
        /// a whole BGP message, header included
        bgp: &'a [u8],
    },
    StatisticsReport {
        peer: PeerHeader,
        stats: &'a [u8],
    },
    PeerDown {
        peer: PeerHeader,
        reason: u8,
    },
    PeerUp {
        peer: PeerHeader,
        /// OPEN bodies, after the BGP header
        sent_open: &'a [u8],
        received_open: &'a [u8],
        info: &'a [u8],
    },
    Initiation {
        info: &'a [u8],
    },
    Termination {
        info: &'a [u8],
    },
    RouteMirroring {
        peer: PeerHeader,
    },
    Other {
        kind: u8,
    },
}

impl<'a> Message<'a> {
    /// Reads one framed message, common header included.
    pub fn parse(frame: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(frame);
        let header = CommonHeader::parse(r.take(COMMON_HEADER_LEN, "bmp header truncated")?)?;
        if header.version != VERSION {
            return Err(WireError("unsupported bmp version"));
        }
        if header.length as usize != frame.len() {
            return Err(WireError("bmp length does not match the frame"));
        }
        Ok(match header.kind {
            ROUTE_MONITORING => Message::RouteMonitoring {
                peer: PeerHeader::parse(&mut r)?,
                bgp: r.rest(),
            },
            STATISTICS_REPORT => Message::StatisticsReport {
                peer: PeerHeader::parse(&mut r)?,
                stats: r.rest(),
            },
            PEER_DOWN => Message::PeerDown {
                peer: PeerHeader::parse(&mut r)?,
                reason: r.u8("peer down truncated")?,
            },
            PEER_UP => {
                let peer = PeerHeader::parse(&mut r)?;
                let _local_address = r.take(16, "peer up truncated")?;
                let _local_port = r.u16("peer up truncated")?;
                let _remote_port = r.u16("peer up truncated")?;
                let sent_open = open(&mut r)?;
                let received_open = open(&mut r)?;
                Message::PeerUp {
                    peer,
                    sent_open,
                    received_open,
                    info: r.rest(),
                }
            }
            INITIATION => Message::Initiation { info: r.rest() },
            TERMINATION => Message::Termination { info: r.rest() },
            ROUTE_MIRRORING => Message::RouteMirroring {
                peer: PeerHeader::parse(&mut r)?,
            },
            kind => Message::Other { kind },
        })
    }
}

fn open<'a>(r: &mut Reader<'a>) -> Result<&'a [u8]> {
    match bgp_message(r)? {
        (BGP_OPEN, body) => Ok(body),
        _ => Err(WireError("peer up without an open message")),
    }
}

/// Type-length-value entries with 16-bit types and lengths: information TLVs
/// (Initiation, Termination, Peer Up) and statistics counters.
pub fn tlvs(bytes: &[u8]) -> impl Iterator<Item = Result<(u16, &[u8])>> {
    let mut r = Reader::new(bytes);
    std::iter::from_fn(move || {
        if r.is_empty() {
            return None;
        }
        let item = (|| {
            let kind = r.u16("tlv truncated")?;
            let len = r.u16("tlv truncated")? as usize;
            Ok((kind, r.take(len, "tlv truncated")?))
        })();
        if item.is_err() {
            r = Reader::new(&[]);
        }
        Some(item)
    })
}

/// The Loc-RIB route counts of a statistics report, for comparing with the
/// table's own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocRibCounts {
    pub total: Option<u64>,
    pub per_family: [Option<u64>; 2],
}

pub fn loc_rib_counts(stats: &[u8]) -> Result<LocRibCounts> {
    let mut r = Reader::new(stats);
    let _count = r.u32("statistics truncated")?;
    let mut counts = LocRibCounts::default();
    for tlv in tlvs(r.rest()) {
        let (kind, value) = tlv?;
        let mut v = Reader::new(value);
        match kind {
            STAT_LOC_RIB => counts.total = Some(v.u64("statistic truncated")?),
            STAT_LOC_RIB_PER_AFI => {
                let afi = v.u16("statistic truncated")?;
                let safi = v.u8("statistic truncated")?;
                let n = v.u64("statistic truncated")?;
                if let Some(family) = Afi::unicast(afi, safi) {
                    counts.per_family[family.index()] = Some(n);
                }
            }
            _ => {}
        }
    }
    Ok(counts)
}

/// The text of a non-zero peer distinguisher (RFC 4364 §4.2): `asn:n` for
/// type 0, `a.b.c.d:n` for type 1, `asn:n` for type 2 (`2:asn:n` when the ASN
/// would fit type 0, so the two never collide). Anything else, such as the
/// locally defined ids RFC 9069 allows, is its 16 hex digits.
/// `None` for the global instance (all zero).
pub fn distinguisher_text(d: &[u8; 8]) -> Option<String> {
    if d == &[0; 8] {
        return None;
    }
    let mut r = Reader::new(d);
    let mut out = String::new();
    let kind = r.u16("").unwrap_or(u16::MAX);
    let _ = match kind {
        0 => {
            let asn = r.u16("").unwrap_or_default();
            let n = r.u32("").unwrap_or_default();
            write!(out, "{asn}:{n}")
        }
        1 => {
            let ip = Ipv4Addr::from(r.u32("").unwrap_or_default());
            let n = r.u16("").unwrap_or_default();
            write!(out, "{ip}:{n}")
        }
        2 => {
            let asn = r.u32("").unwrap_or_default();
            let n = r.u16("").unwrap_or_default();
            if asn <= u16::MAX as u32 {
                write!(out, "2:{asn}:{n}")
            } else {
                write!(out, "{asn}:{n}")
            }
        }
        _ => {
            for b in d {
                let _ = write!(out, "{b:02x}");
            }
            Ok(())
        }
    };
    Some(out)
}

/// Families a statistics AFI refers to, for the tests' builders.
#[cfg(test)]
pub fn family_afi(family: Afi) -> u16 {
    match family {
        Afi::V4 => super::AFI_IPV4,
        Afi::V6 => super::AFI_IPV6,
    }
}

/// Test helpers that build BMP messages byte by byte.
#[cfg(test)]
pub mod build {
    use super::*;

    pub fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![VERSION];
        out.extend_from_slice(&((COMMON_HEADER_LEN + body.len()) as u32).to_be_bytes());
        out.push(kind);
        out.extend_from_slice(body);
        out
    }

    pub fn peer_header(
        peer_type: u8,
        flags: u8,
        rd: [u8; 8],
        addr: IpAddr,
        asn: u32,
        id: Ipv4Addr,
    ) -> Vec<u8> {
        let mut out = vec![peer_type, flags];
        out.extend_from_slice(&rd);
        match addr {
            IpAddr::V4(a) => {
                out.extend_from_slice(&[0; 12]);
                out.extend_from_slice(&a.octets());
            }
            IpAddr::V6(a) => out.extend_from_slice(&a.octets()),
        }
        out.extend_from_slice(&asn.to_be_bytes());
        out.extend_from_slice(&id.octets());
        out.extend_from_slice(&[0; 8]);
        out
    }

    /// A Loc-RIB per-peer header for the instance `rd` (zero for global).
    pub fn loc_rib(rd: [u8; 8]) -> Vec<u8> {
        peer_header(
            PEER_LOC_RIB,
            0,
            rd,
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            64500,
            Ipv4Addr::new(192, 0, 2, 254),
        )
    }

    pub fn rd0(asn: u16, n: u32) -> [u8; 8] {
        let mut rd = [0; 8];
        rd[2..4].copy_from_slice(&asn.to_be_bytes());
        rd[4..].copy_from_slice(&n.to_be_bytes());
        rd
    }

    fn bgp(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![0xff; 16];
        out.extend_from_slice(&((super::super::BGP_HEADER_LEN + body.len()) as u16).to_be_bytes());
        out.push(kind);
        out.extend_from_slice(body);
        out
    }

    pub fn initiation(sys_name: &str) -> Vec<u8> {
        let mut body = 2u16.to_be_bytes().to_vec();
        body.extend_from_slice(&(sys_name.len() as u16).to_be_bytes());
        body.extend_from_slice(sys_name.as_bytes());
        frame(INITIATION, &body)
    }

    pub fn termination(reason: u16) -> Vec<u8> {
        let mut body = 1u16.to_be_bytes().to_vec();
        body.extend_from_slice(&2u16.to_be_bytes());
        body.extend_from_slice(&reason.to_be_bytes());
        frame(TERMINATION, &body)
    }

    /// A Peer Up whose sent and received OPEN are both `open` (as a Loc-RIB
    /// peer's fabricated one is).
    pub fn peer_up(peer: &[u8], open: &[u8]) -> Vec<u8> {
        let mut body = peer.to_vec();
        body.extend_from_slice(&[0; 16]);
        body.extend_from_slice(&179u16.to_be_bytes());
        body.extend_from_slice(&0u16.to_be_bytes());
        body.extend_from_slice(&bgp(BGP_OPEN, open));
        body.extend_from_slice(&bgp(BGP_OPEN, open));
        frame(PEER_UP, &body)
    }

    pub fn peer_down(peer: &[u8], reason: u8) -> Vec<u8> {
        let mut body = peer.to_vec();
        body.push(reason);
        frame(PEER_DOWN, &body)
    }

    /// A Route Monitoring message around an UPDATE body.
    pub fn route_monitoring(peer: &[u8], update_body: &[u8]) -> Vec<u8> {
        let mut body = peer.to_vec();
        body.extend_from_slice(&bgp(super::super::BGP_UPDATE, update_body));
        frame(ROUTE_MONITORING, &body)
    }

    pub fn stats(peer: &[u8], counters: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut body = peer.to_vec();
        body.extend_from_slice(&(counters.len() as u32).to_be_bytes());
        for (kind, value) in counters {
            body.extend_from_slice(&kind.to_be_bytes());
            body.extend_from_slice(&(value.len() as u16).to_be_bytes());
            body.extend_from_slice(value);
        }
        frame(STATISTICS_REPORT, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;
    use crate::bgp::wire::open::{Open, build_open};

    #[test]
    fn distinguishers_render_like_rds() {
        assert_eq!(distinguisher_text(&[0; 8]), None);
        assert_eq!(
            distinguisher_text(&rd0(65017, 101)).as_deref(),
            Some("65017:101")
        );
        let ip = [0, 1, 192, 0, 2, 1, 0, 7];
        assert_eq!(distinguisher_text(&ip).as_deref(), Some("192.0.2.1:7"));
        let big = [0, 2, 0xfa, 0x56, 0xea, 0x00, 0, 9];
        assert_eq!(distinguisher_text(&big).as_deref(), Some("4200000000:9"));
        let small = [0, 2, 0, 0, 0xfd, 0xe8, 0, 9];
        assert_eq!(distinguisher_text(&small).as_deref(), Some("2:65000:9"));
        let local = [0x80, 0, 0, 0, 0, 0, 0, 1];
        assert_eq!(
            distinguisher_text(&local).as_deref(),
            Some("8000000000000001")
        );
    }

    #[test]
    fn a_peer_up_carries_both_opens() {
        let open = build_open(64500, Ipv4Addr::new(192, 0, 2, 254), &[]);
        let frame = peer_up(&loc_rib(rd0(64500, 1)), &open);
        let Message::PeerUp {
            peer,
            sent_open,
            received_open,
            info,
        } = Message::parse(&frame).unwrap()
        else {
            panic!("not a peer up");
        };
        assert!(peer.is_loc_rib());
        assert!(peer.asn4());
        assert_eq!(peer.distinguisher, rd0(64500, 1));
        assert_eq!(Open::parse(received_open).unwrap().asn, 64500);
        assert_eq!(sent_open, received_open);
        assert!(info.is_empty());
    }

    #[test]
    fn the_peer_address_follows_the_v_flag() {
        let v4 = peer_header(
            PEER_GLOBAL,
            0,
            [0; 8],
            "192.0.2.9".parse().unwrap(),
            1,
            Ipv4Addr::LOCALHOST,
        );
        let frame = peer_down(&v4, 4);
        let Message::PeerDown { peer, reason } = Message::parse(&frame).unwrap() else {
            panic!("not a peer down");
        };
        assert_eq!(peer.address, "192.0.2.9".parse::<IpAddr>().unwrap());
        assert_eq!(reason, 4);
        let a2 = peer_header(
            PEER_GLOBAL,
            FLAG_AS2,
            [0; 8],
            "192.0.2.9".parse().unwrap(),
            1,
            Ipv4Addr::LOCALHOST,
        );
        let Message::PeerDown { peer, .. } = Message::parse(&peer_down(&a2, 4)).unwrap() else {
            panic!("not a peer down");
        };
        assert!(!peer.asn4());
    }

    #[test]
    fn statistics_yield_the_loc_rib_counts() {
        let mut per_afi = family_afi(Afi::V6).to_be_bytes().to_vec();
        per_afi.push(1);
        per_afi.extend_from_slice(&42u64.to_be_bytes());
        let frame = stats(
            &loc_rib([0; 8]),
            &[
                (0, 5u32.to_be_bytes().to_vec()),
                (STAT_LOC_RIB, 100u64.to_be_bytes().to_vec()),
                (STAT_LOC_RIB_PER_AFI, per_afi),
            ],
        );
        let Message::StatisticsReport { stats, .. } = Message::parse(&frame).unwrap() else {
            panic!("not a statistics report");
        };
        let counts = loc_rib_counts(stats).unwrap();
        assert_eq!(counts.total, Some(100));
        assert_eq!(counts.per_family, [None, Some(42)]);
    }

    #[test]
    fn a_bad_frame_is_an_error() {
        let mut f = initiation("r1");
        f[0] = 4;
        assert!(Message::parse(&f).is_err(), "version");
        let f = initiation("r1");
        assert!(
            Message::parse(&f[..f.len() - 1]).is_err(),
            "length mismatch"
        );
        let short = frame(ROUTE_MONITORING, &[0; 10]);
        assert!(Message::parse(&short).is_err(), "peer header truncated");
        assert!(matches!(
            Message::parse(&frame(77, &[])).unwrap(),
            Message::Other { kind: 77 }
        ));
        // a peer up whose second message is not an OPEN
        let open = build_open(1, Ipv4Addr::LOCALHOST, &[]);
        let mut f = peer_up(&loc_rib([0; 8]), &open);
        let second_type_at = COMMON_HEADER_LEN + PEER_HEADER_LEN + 20 + 19 + open.len() + 18;
        f[second_type_at] = 2;
        assert!(Message::parse(&f).is_err());
    }

    #[test]
    fn tlvs_stop_at_a_truncated_entry() {
        let mut bytes = vec![0, 2, 0, 2, b'r', b'1'];
        bytes.extend_from_slice(&[0, 1, 0, 9, b'x']);
        let items: Vec<_> = tlvs(&bytes).collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], Ok((2, &b"r1"[..])));
        assert!(items[1].is_err());
    }
}
