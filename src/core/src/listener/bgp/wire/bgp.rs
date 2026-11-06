// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The BGP messages a passive session sends or reads besides OPEN and UPDATE:
//! KEEPALIVE, NOTIFICATION (RFC 4271 §4.5, RFC 9003) and ROUTE-REFRESH
//! (RFC 2918).

use super::{BGP_HEADER_LEN, Reader, Result};

pub const BGP_NOTIFICATION: u8 = 3;
pub const BGP_KEEPALIVE: u8 = 4;
pub const BGP_ROUTE_REFRESH: u8 = 5;

/// The longest message without RFC 8654 extended messages.
pub const MAX_MESSAGE: usize = 4096;
/// The longest message with them.
pub const MAX_EXTENDED_MESSAGE: usize = 65535;

/// NOTIFICATION error codes (RFC 4271 §4.5, RFC 7313).
pub mod code {
    pub const MESSAGE_HEADER: u8 = 1;
    pub const OPEN_MESSAGE: u8 = 2;
    pub const UPDATE_MESSAGE: u8 = 3;
    pub const HOLD_TIMER_EXPIRED: u8 = 4;
    pub const FSM: u8 = 5;
    pub const CEASE: u8 = 6;
}

/// Subcodes of the errors this session raises.
pub mod subcode {
    // message header
    pub const CONNECTION_NOT_SYNCHRONIZED: u8 = 1;
    pub const BAD_MESSAGE_LENGTH: u8 = 2;
    pub const BAD_MESSAGE_TYPE: u8 = 3;
    // open message
    pub const UNSPECIFIC: u8 = 0;
    pub const UNSUPPORTED_VERSION: u8 = 1;
    pub const BAD_PEER_AS: u8 = 2;
    pub const BAD_BGP_IDENTIFIER: u8 = 3;
    pub const UNSUPPORTED_OPTIONAL_PARAMETER: u8 = 4;
    pub const UNACCEPTABLE_HOLD_TIME: u8 = 6;
    // update message
    pub const MALFORMED_ATTRIBUTE_LIST: u8 = 1;
    pub const MISSING_WELL_KNOWN_ATTRIBUTE: u8 = 3;
    pub const ATTRIBUTE_FLAGS_ERROR: u8 = 4;
    pub const ATTRIBUTE_LENGTH_ERROR: u8 = 5;
    pub const INVALID_ORIGIN_ATTRIBUTE: u8 = 6;
    pub const OPTIONAL_ATTRIBUTE_ERROR: u8 = 9;
    pub const INVALID_NETWORK_FIELD: u8 = 10;
    pub const MALFORMED_AS_PATH: u8 = 11;
    // finite state machine (RFC 6608)
    pub const UNEXPECTED_IN_OPEN_SENT: u8 = 1;
    pub const UNEXPECTED_IN_OPEN_CONFIRM: u8 = 2;
    pub const UNEXPECTED_IN_ESTABLISHED: u8 = 3;
    // cease (RFC 4486)
    pub const ADMINISTRATIVE_SHUTDOWN: u8 = 2;
    pub const ADMINISTRATIVE_RESET: u8 = 4;
    pub const CONNECTION_REJECTED: u8 = 5;
    pub const OTHER_CONFIGURATION_CHANGE: u8 = 6;
    pub const CONNECTION_COLLISION: u8 = 7;
}

/// The label of each NOTIFICATION error code, by [`code_index`].
pub const CODE_NAMES: [&str; 8] = [
    "other",
    "message_header",
    "open_message",
    "update_message",
    "hold_timer_expired",
    "fsm",
    "cease",
    "route_refresh",
];

/// The index of `code` in [`CODE_NAMES`]; unknown codes are "other".
pub fn code_index(code: u8) -> usize {
    match code {
        1..=7 => code as usize,
        _ => 0,
    }
}

/// A whole BGP message: marker, length, type and `body`. The body must fit
/// the extended maximum.
pub fn message(kind: u8, body: &[u8]) -> Vec<u8> {
    let len = BGP_HEADER_LEN + body.len();
    debug_assert!(len <= MAX_EXTENDED_MESSAGE);
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&[0xff; 16]);
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.push(kind);
    out.extend_from_slice(body);
    out
}

pub fn keepalive() -> Vec<u8> {
    message(BGP_KEEPALIVE, &[])
}

pub fn notification(code: u8, subcode: u8, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(2 + data.len());
    body.extend_from_slice(&[code, subcode]);
    body.extend_from_slice(data);
    message(BGP_NOTIFICATION, &body)
}

/// The data of a Cease / Administrative Shutdown or Reset carrying a
/// shutdown communication (RFC 9003): a length byte and UTF-8 text, cut to
/// 255 bytes on a character boundary.
pub fn shutdown_communication(text: &str) -> Vec<u8> {
    let mut end = text.len().min(255);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = Vec::with_capacity(1 + end);
    out.push(end as u8);
    out.extend_from_slice(&text.as_bytes()[..end]);
    out
}

/// The End-of-RIB marker of a family (RFC 4724 §2): an UPDATE that announces
/// and withdraws nothing, empty for IPv4 unicast, otherwise an empty
/// MP_UNREACH_NLRI. The only UPDATE patrolli ever sends; it carries no
/// route.
pub fn end_of_rib(afi: u16, safi: u8) -> Vec<u8> {
    if (afi, safi) == (super::AFI_IPV4, super::SAFI_UNICAST) {
        return message(super::BGP_UPDATE, &[0, 0, 0, 0]);
    }
    let afi = afi.to_be_bytes();
    // no withdrawn routes; 6 bytes of attributes: optional, MP_UNREACH_NLRI,
    // length 3, AFI, SAFI
    let body = [0, 0, 0, 6, 0x80, 15, 3, afi[0], afi[1], safi];
    message(super::BGP_UPDATE, &body)
}

pub fn route_refresh(afi: u16, safi: u8) -> Vec<u8> {
    let afi = afi.to_be_bytes();
    message(BGP_ROUTE_REFRESH, &[afi[0], afi[1], 0, safi])
}

/// A received NOTIFICATION.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notification<'a> {
    pub code: u8,
    pub subcode: u8,
    pub data: &'a [u8],
}

impl<'a> Notification<'a> {
    /// Reads a NOTIFICATION body (after the BGP header).
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(body);
        let code = r.u8("notification truncated")?;
        let subcode = r.u8("notification truncated")?;
        Ok(Self {
            code,
            subcode,
            data: r.rest(),
        })
    }

    pub fn code_name(&self) -> &'static str {
        CODE_NAMES[code_index(self.code)]
    }

    /// The RFC 9003 shutdown communication of a Cease / Administrative
    /// Shutdown or Reset, if it carries valid UTF-8 text.
    pub fn shutdown_communication(&self) -> Option<&'a str> {
        if self.code != code::CEASE
            || !matches!(
                self.subcode,
                subcode::ADMINISTRATIVE_SHUTDOWN | subcode::ADMINISTRATIVE_RESET
            )
        {
            return None;
        }
        let mut r = Reader::new(self.data);
        let len = r.u8("").ok()? as usize;
        let text = r.take(len, "").ok()?;
        std::str::from_utf8(text).ok().filter(|t| !t.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listener::bgp::wire::bgp_message;

    #[test]
    fn notifications_round_trip_with_their_shutdown_text() {
        let text = "maintenance until 18:00 ✓";
        let msg = notification(
            code::CEASE,
            subcode::ADMINISTRATIVE_SHUTDOWN,
            &shutdown_communication(text),
        );
        let (kind, body) = bgp_message(&mut Reader::new(&msg)).unwrap();
        assert_eq!(kind, BGP_NOTIFICATION);
        let n = Notification::parse(body).unwrap();
        assert_eq!((n.code, n.subcode), (6, 2));
        assert_eq!(n.code_name(), "cease");
        assert_eq!(n.shutdown_communication(), Some(text));

        let other = Notification::parse(&[4, 0]).unwrap();
        assert_eq!(other.code_name(), "hold_timer_expired");
        assert_eq!(other.shutdown_communication(), None);
        assert_eq!(Notification::parse(&[99, 1]).unwrap().code_name(), "other");
        assert!(Notification::parse(&[6]).is_err());
        // a length past the data is no text, not a panic
        assert_eq!(
            Notification::parse(&[6, 2, 9, b'a'])
                .unwrap()
                .shutdown_communication(),
            None
        );
    }

    #[test]
    fn a_long_shutdown_text_is_cut_on_a_character_boundary() {
        let text = "é".repeat(200);
        let data = shutdown_communication(&text);
        assert_eq!(data[0], 254);
        assert!(std::str::from_utf8(&data[1..]).is_ok());
    }

    #[test]
    fn keepalive_and_route_refresh_are_whole_messages() {
        assert_eq!(keepalive().len(), BGP_HEADER_LEN);
        let rr = route_refresh(2, 1);
        let (kind, body) = bgp_message(&mut Reader::new(&rr)).unwrap();
        assert_eq!((kind, body), (BGP_ROUTE_REFRESH, &[0, 2, 0, 1][..]));
    }
}
