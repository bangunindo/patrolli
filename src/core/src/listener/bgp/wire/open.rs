// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The OPEN message (RFC 4271 §4.2) and the capabilities (RFC 5492) that
//! change how UPDATEs are read.

use super::{Afi, Reader, Result};
use std::net::Ipv4Addr;

/// Capability codes read here.
const CAP_MULTIPROTOCOL: u8 = 1;
const CAP_EXTENDED_MESSAGE: u8 = 6;
const CAP_FOUR_OCTET_AS: u8 = 65;
const CAP_ADD_PATH: u8 = 69;
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
        if params_len == 255 {
            // RFC 9072 §2: the first parameter type is 255 and a two-byte
            // length follows
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

/// Whether routes received under `open` carry path ids for `family`: the
/// sender said it sends them (RFC 7911 §4).
///
/// For a Loc-RIB peer (RFC 9069 §5.2) the OPEN is fabricated and describes
/// the router's own Loc-RIB, so its send bit is the answer. For a real
/// neighbour both directions would have to agree; those peers are not read.
pub fn sends_path_ids(open: &Open, family: Afi) -> bool {
    open.add_path[family.index()] & ADD_PATH_SEND != 0
}

/// Test helper: an OPEN body with the given capabilities, each `(code, value)`.
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

/// Test helper: the capability values for the usual cases.
#[cfg(test)]
pub mod caps {
    use super::*;
    pub use crate::listener::bgp::wire::{AFI_IPV4, AFI_IPV6, SAFI_UNICAST};

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
        assert!(sends_path_ids(&open, Afi::V4));
        assert!(!sends_path_ids(&open, Afi::V6), "receive only");
        assert!(open.extended_message);
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
