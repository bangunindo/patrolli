// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The OPEN message (RFC 4271 §4.2) and the capabilities (RFC 5492) that
//! change how UPDATEs are read.

use super::{Afi, Reader, Result};
use std::net::Ipv4Addr;

/// Capability codes read or sent here.
const CAP_MULTIPROTOCOL: u8 = 1;
const CAP_ROUTE_REFRESH: u8 = 2;
const CAP_EXTENDED_NEXT_HOP: u8 = 5;
const CAP_EXTENDED_MESSAGE: u8 = 6;
const CAP_GRACEFUL_RESTART: u8 = 64;
const CAP_FOUR_OCTET_AS: u8 = 65;
const CAP_ADD_PATH: u8 = 69;

/// The 2-octet AS a 4-octet speaker puts in "My Autonomous System" when its AS
/// does not fit (RFC 6793).
pub const AS_TRANS: u16 = 23456;
/// Optional parameter type of a capabilities list.
const PARAM_CAPABILITIES: u8 = 2;
/// RFC 9072: a non-extended parameter length of 255 followed by this type
/// switches to two-byte lengths.
const PARAM_EXTENDED: u8 = 255;

/// ADD-PATH send/receive (RFC 7911 §4).
pub const ADD_PATH_RECEIVE: u8 = 1;
pub const ADD_PATH_SEND: u8 = 2;

/// What an OPEN says about the UPDATEs that follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Open {
    /// the 4-octet AS when advertised, otherwise "My Autonomous System"
    pub asn: u32,
    pub bgp_id: Ipv4Addr,
    pub hold_time: u16,
    pub four_octet_as: bool,
    /// unicast families from the multiprotocol capability; without any
    /// multiprotocol capability, IPv4 unicast is implied (RFC 4760 §8)
    pub families: [bool; 2],
    /// the ADD-PATH send/receive bits per unicast family, 0 when absent
    pub add_path: [u8; 2],
    pub extended_message: bool,
    /// the sender accepts ROUTE-REFRESH (RFC 2918)
    pub route_refresh: bool,
    /// the restart time of a Graceful Restart capability (RFC 4724)
    pub graceful_restart: Option<u16>,
    /// the first optional parameter that is not a capabilities list, which a
    /// BGP session refuses (RFC 4271 §6.2)
    pub unknown_parameter: Option<u8>,
}

impl Open {
    /// Reads an OPEN body (after the BGP header).
    pub fn parse(body: &[u8]) -> Result<Self> {
        let mut r = Reader::new(body);
        let _version = r.u8("open truncated")?;
        let my_as = r.u16("open truncated")?;
        let hold_time = r.u16("open truncated")?;
        let bgp_id = Ipv4Addr::from(r.u32("open truncated")?);
        let mut params_len = r.u8("open truncated")? as usize;
        let mut extended = false;
        if params_len != 0 {
            // RFC 9072 §2: a first parameter type of 255 means the extended
            // encoding, whatever the one-byte length says (it is then
            // ignored), and a two-byte length follows
            let mut peek = r;
            if peek.u8("open truncated")? == PARAM_EXTENDED {
                r = peek;
                params_len = r.u16("open truncated")? as usize;
                extended = true;
            }
        }
        let mut params = r.sub(params_len, "open parameters truncated")?;

        let mut open = Open {
            asn: my_as.into(),
            bgp_id,
            hold_time,
            four_octet_as: false,
            families: [false; 2],
            add_path: [0; 2],
            extended_message: false,
            route_refresh: false,
            graceful_restart: None,
            unknown_parameter: None,
        };
        let mut multiprotocol = false;
        while !params.is_empty() {
            let kind = params.u8("open parameter truncated")?;
            let len = if extended {
                params.u16("open parameter truncated")? as usize
            } else {
                params.u8("open parameter truncated")? as usize
            };
            let mut value = params.sub(len, "open parameter truncated")?;
            if kind != PARAM_CAPABILITIES {
                // RFC 4271 §6.2; a type 255 here is one too (RFC 9072 §3)
                open.unknown_parameter.get_or_insert(kind);
                continue;
            }
            while !value.is_empty() {
                let code = value.u8("capability truncated")?;
                let len = value.u8("capability truncated")? as usize;
                let mut cap = value.sub(len, "capability truncated")?;
                match code {
                    CAP_MULTIPROTOCOL => {
                        multiprotocol = true;
                        let afi = cap.u16("multiprotocol capability truncated")?;
                        let _reserved = cap.u8("multiprotocol capability truncated")?;
                        let safi = cap.u8("multiprotocol capability truncated")?;
                        if let Some(family) = Afi::unicast(afi, safi) {
                            open.families[family.index()] = true;
                        }
                    }
                    CAP_FOUR_OCTET_AS => {
                        open.asn = cap.u32("four-octet as capability truncated")?;
                        open.four_octet_as = true;
                    }
                    CAP_ADD_PATH => {
                        while !cap.is_empty() {
                            let afi = cap.u16("add-path capability truncated")?;
                            let safi = cap.u8("add-path capability truncated")?;
                            let mode = cap.u8("add-path capability truncated")?;
                            if let Some(family) = Afi::unicast(afi, safi) {
                                open.add_path[family.index()] = mode & 0b11;
                            }
                        }
                    }
                    CAP_EXTENDED_MESSAGE => open.extended_message = true,
                    CAP_ROUTE_REFRESH => open.route_refresh = true,
                    CAP_GRACEFUL_RESTART => {
                        let flags_time = cap.u16("graceful restart capability truncated")?;
                        open.graceful_restart = Some(flags_time & 0x0fff);
                    }
                    _ => {}
                }
            }
        }
        if !multiprotocol {
            open.families[Afi::V4.index()] = true;
        }
        Ok(open)
    }
}

/// Whether Loc-RIB routes under the fabricated `open` of a Loc-RIB peer carry
/// path ids for `family`.
///
/// RFC 9069 §5.2: the capability's send/receive intent is ignored, its
/// presence alone means the Loc-RIB carries additional paths. Reading only the
/// send bit would take a router's receive-only capability as no path ids and
/// misread every path id as prefix bytes (a path id of 0 reads as `/0`).
pub fn loc_rib_path_ids(open: &Open, family: Afi) -> bool {
    open.add_path[family.index()] != 0
}

/// An OPEN body (after the BGP header) with the given capabilities, each
/// `(code, value)`. An AS above 65535 goes out as [`AS_TRANS`]; the 4-octet AS
/// capability carries it.
pub fn open_body(asn: u32, hold_time: u16, bgp_id: Ipv4Addr, caps: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut list = Vec::new();
    for (code, value) in caps {
        list.push(*code);
        list.push(value.len() as u8);
        list.extend_from_slice(value);
    }
    let my_as = u16::try_from(asn).unwrap_or(AS_TRANS);
    let mut out = vec![4];
    out.extend_from_slice(&my_as.to_be_bytes());
    out.extend_from_slice(&hold_time.to_be_bytes());
    out.extend_from_slice(&bgp_id.octets());
    // one capabilities parameter; ours always fit a one-byte length
    out.push(list.len() as u8 + 2);
    out.push(PARAM_CAPABILITIES);
    out.push(list.len() as u8);
    out.extend_from_slice(&list);
    out
}

/// Test helper: an OPEN body with the given capabilities, each in a parameter
/// of its own, as some routers send them.
#[cfg(test)]
pub fn build_open(my_as: u16, bgp_id: Ipv4Addr, caps: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut params = Vec::new();
    for (code, value) in caps {
        let mut cap = vec![*code, value.len() as u8];
        cap.extend_from_slice(value);
        params.push(PARAM_CAPABILITIES);
        params.push(cap.len() as u8);
        params.extend_from_slice(&cap);
    }
    let mut out = vec![4];
    out.extend_from_slice(&my_as.to_be_bytes());
    out.extend_from_slice(&180u16.to_be_bytes());
    out.extend_from_slice(&bgp_id.octets());
    out.push(params.len() as u8);
    out.extend_from_slice(&params);
    out
}

/// Capability values, each `(code, value)`.
pub mod caps {
    use super::*;
    pub use crate::bgp::wire::{AFI_IPV4, AFI_IPV6, SAFI_UNICAST};

    pub fn multiprotocol(afi: u16, safi: u8) -> (u8, Vec<u8>) {
        let mut v = afi.to_be_bytes().to_vec();
        v.extend_from_slice(&[0, safi]);
        (CAP_MULTIPROTOCOL, v)
    }

    pub fn four_octet(asn: u32) -> (u8, Vec<u8>) {
        (CAP_FOUR_OCTET_AS, asn.to_be_bytes().to_vec())
    }

    pub fn add_path(families: &[(u16, u8, u8)]) -> (u8, Vec<u8>) {
        let mut v = Vec::new();
        for (afi, safi, mode) in families {
            v.extend_from_slice(&afi.to_be_bytes());
            v.extend_from_slice(&[*safi, *mode]);
        }
        (CAP_ADD_PATH, v)
    }

    pub fn v4_v6() -> [(u8, Vec<u8>); 2] {
        [
            multiprotocol(AFI_IPV4, SAFI_UNICAST),
            multiprotocol(AFI_IPV6, SAFI_UNICAST),
        ]
    }

    pub fn route_refresh() -> (u8, Vec<u8>) {
        (CAP_ROUTE_REFRESH, Vec::new())
    }

    pub fn extended_message() -> (u8, Vec<u8>) {
        (CAP_EXTENDED_MESSAGE, Vec::new())
    }

    /// RFC 8950: IPv4 unicast routes may carry an IPv6 next hop.
    pub fn extended_next_hop_v4_over_v6() -> (u8, Vec<u8>) {
        let mut v = AFI_IPV4.to_be_bytes().to_vec();
        v.extend_from_slice(&(SAFI_UNICAST as u16).to_be_bytes());
        v.extend_from_slice(&AFI_IPV6.to_be_bytes());
        (CAP_EXTENDED_NEXT_HOP, v)
    }

    /// RFC 4724 §3 with no families: the sender preserves no forwarding
    /// state of its own, but follows the receiving speaker's procedures and
    /// understands End-of-RIB.
    pub fn graceful_restart_receiver() -> (u8, Vec<u8>) {
        (CAP_GRACEFUL_RESTART, vec![0, 0])
    }

    /// A Graceful Restart capability with a restart time, as a router sends.
    #[cfg(test)]
    pub fn graceful_restart(restart_time: u16) -> (u8, Vec<u8>) {
        (
            CAP_GRACEFUL_RESTART,
            (restart_time & 0x0fff).to_be_bytes().to_vec(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::caps::{AFI_IPV4, AFI_IPV6, SAFI_UNICAST};
    use super::*;

    #[test]
    fn capabilities_set_families_asn_and_add_path() {
        let id = Ipv4Addr::new(192, 0, 2, 1);
        let mut list = caps::v4_v6().to_vec();
        list.push(caps::four_octet(4_200_000_000));
        list.push(caps::add_path(&[
            (AFI_IPV4, SAFI_UNICAST, ADD_PATH_SEND | ADD_PATH_RECEIVE),
            (AFI_IPV6, SAFI_UNICAST, ADD_PATH_RECEIVE),
            (AFI_IPV4, 128, ADD_PATH_SEND),
        ]));
        list.push((CAP_EXTENDED_MESSAGE, vec![]));
        let open = Open::parse(&build_open(23456, id, &list)).unwrap();
        assert_eq!(open.asn, 4_200_000_000);
        assert!(open.four_octet_as);
        assert_eq!(open.bgp_id, id);
        assert_eq!(open.families, [true, true]);
        assert_eq!(
            open.add_path,
            [ADD_PATH_SEND | ADD_PATH_RECEIVE, ADD_PATH_RECEIVE]
        );
        assert!(loc_rib_path_ids(&open, Afi::V4));
        assert!(
            loc_rib_path_ids(&open, Afi::V6),
            "RFC 9069 §5.2: any mode means path ids"
        );
        assert!(open.extended_message);
    }

    #[test]
    fn our_open_reads_back_as_sent() {
        let id = Ipv4Addr::new(192, 0, 2, 10);
        let list = [
            caps::multiprotocol(AFI_IPV4, SAFI_UNICAST),
            caps::four_octet(4_200_000_000),
            caps::route_refresh(),
            caps::extended_message(),
            caps::extended_next_hop_v4_over_v6(),
            caps::graceful_restart_receiver(),
        ];
        let body = open_body(4_200_000_000, 90, id, &list);
        assert_eq!(&body[1..3], &AS_TRANS.to_be_bytes());
        let open = Open::parse(&body).unwrap();
        assert_eq!(open.asn, 4_200_000_000);
        assert_eq!(open.hold_time, 90);
        assert_eq!(open.bgp_id, id);
        assert_eq!(open.families, [true, false]);
        assert!(open.route_refresh && open.extended_message && open.four_octet_as);
        assert_eq!(open.graceful_restart, Some(0));
        assert_eq!(open.add_path, [0, 0]);

        let small = Open::parse(&open_body(64500, 0, id, &[])).unwrap();
        assert_eq!((small.asn, small.hold_time), (64500, 0));
        let gr = Open::parse(&build_open(64500, id, &[caps::graceful_restart(120)])).unwrap();
        assert_eq!(gr.graceful_restart, Some(120));
    }

    #[test]
    fn without_multiprotocol_ipv4_unicast_is_implied() {
        let open = Open::parse(&build_open(64500, Ipv4Addr::LOCALHOST, &[])).unwrap();
        assert_eq!(open.asn, 64500);
        assert!(!open.four_octet_as);
        assert_eq!(open.families, [true, false]);
        assert_eq!(open.add_path, [0, 0]);

        let v6_only = [caps::multiprotocol(AFI_IPV6, SAFI_UNICAST)];
        let open = Open::parse(&build_open(64500, Ipv4Addr::LOCALHOST, &v6_only)).unwrap();
        assert_eq!(open.families, [false, true]);
    }

    #[test]
    fn extended_parameter_lengths_are_read() {
        // RFC 9072: length 255, type 255, then a two-byte length and
        // parameters with two-byte lengths
        let cap = [CAP_FOUR_OCTET_AS, 4, 0, 1, 0, 0];
        let mut body = vec![4, 0, 1, 0, 180, 192, 0, 2, 1, 255, 255];
        let param_len = 1 + 2 + cap.len();
        body.extend_from_slice(&(param_len as u16).to_be_bytes());
        body.push(PARAM_CAPABILITIES);
        body.extend_from_slice(&(cap.len() as u16).to_be_bytes());
        body.extend_from_slice(&cap);
        let open = Open::parse(&body).unwrap();
        assert_eq!(open.asn, 65536);
        assert_eq!(open.unknown_parameter, None);

        // RFC 9072 §2: the one-byte length is ignored once the type is 255,
        // whatever it is
        for len in [1, 9, 254] {
            body[9] = len;
            assert_eq!(Open::parse(&body).unwrap().asn, 65536, "{len}");
        }
    }

    #[test]
    fn a_parameter_that_is_not_capabilities_is_reported() {
        // RFC 4271 §6.2: type 1 (the deprecated authentication), length 0
        let mut body = build_open(64500, Ipv4Addr::LOCALHOST, &caps::v4_v6());
        body[9] += 2;
        body.extend_from_slice(&[1, 0]);
        let open = Open::parse(&body).unwrap();
        assert_eq!(open.unknown_parameter, Some(1));
        assert_eq!(open.families, [true, true], "the rest still read");
    }

    #[test]
    fn a_truncated_open_is_an_error_not_a_panic() {
        let mut list = caps::v4_v6().to_vec();
        list.push(caps::add_path(&[(AFI_IPV4, SAFI_UNICAST, 3)]));
        let body = build_open(64500, Ipv4Addr::LOCALHOST, &list);
        for cut in 0..body.len() {
            assert!(Open::parse(&body[..cut]).is_err(), "cut at {cut}");
        }
        // a capability claiming more than its parameter holds
        let mut bad = body.clone();
        let cap_len_at = 10 + 3;
        bad[cap_len_at] = 200;
        assert!(Open::parse(&bad).is_err());
    }
}
