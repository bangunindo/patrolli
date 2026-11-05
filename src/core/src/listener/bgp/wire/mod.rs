// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Wire decoding of the BGP (RFC 4271) and BMP (RFC 7854, RFC 9069) messages
//! the route table needs, and nothing else.
//!
//! Every decoder borrows from the message it reads, and every read goes
//! through [`Reader`], which checks bounds: peer input must never panic (a
//! panic is fatal to the process). This module depends on `std` only, so a
//! fuzz harness can include it on its own.

pub mod bgp;
pub mod bmp;
pub mod open;
pub mod update;

use std::fmt;

/// Why a message could not be read. The reason is a constant, so it can go
/// into a log field or a counter label as is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireError(pub &'static str);

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for WireError {}

pub type Result<T> = std::result::Result<T, WireError>;

/// AFI (RFC 4760) values this decoder knows.
pub const AFI_IPV4: u16 = 1;
pub const AFI_IPV6: u16 = 2;
/// SAFI unicast; the only one routes are taken from.
pub const SAFI_UNICAST: u8 = 1;

/// The unicast families routes are kept for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Afi {
    V4,
    V6,
}

impl Afi {
    /// The family of an AFI/SAFI pair, if routes are kept for it.
    pub fn unicast(afi: u16, safi: u8) -> Option<Self> {
        match (afi, safi) {
            (AFI_IPV4, SAFI_UNICAST) => Some(Self::V4),
            (AFI_IPV6, SAFI_UNICAST) => Some(Self::V6),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::V4 => 0,
            Self::V6 => 1,
        }
    }

    pub const ALL: [Self; 2] = [Self::V4, Self::V6];
}

/// A bounds-checked cursor over a byte slice.
#[derive(Debug, Clone, Copy)]
pub struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// What is left, consuming the reader.
    pub fn rest(self) -> &'a [u8] {
        self.buf
    }

    pub fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8]> {
        if n > self.buf.len() {
            return Err(WireError(what));
        }
        let (head, tail) = self.buf.split_at(n);
        self.buf = tail;
        Ok(head)
    }

    /// The next `n` bytes as a reader of their own.
    pub fn sub(&mut self, n: usize, what: &'static str) -> Result<Reader<'a>> {
        self.take(n, what).map(Reader::new)
    }

    pub fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N]> {
        let bytes = self.take(N, what)?;
        let mut out = [0; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    pub fn u8(&mut self, what: &'static str) -> Result<u8> {
        Ok(self.array::<1>(what)?[0])
    }

    pub fn u16(&mut self, what: &'static str) -> Result<u16> {
        self.array(what).map(u16::from_be_bytes)
    }

    pub fn u32(&mut self, what: &'static str) -> Result<u32> {
        self.array(what).map(u32::from_be_bytes)
    }

    pub fn u64(&mut self, what: &'static str) -> Result<u64> {
        self.array(what).map(u64::from_be_bytes)
    }
}

/// The 19-byte BGP message header (RFC 4271 §4.1).
pub const BGP_HEADER_LEN: usize = 19;
pub const BGP_OPEN: u8 = 1;
pub const BGP_UPDATE: u8 = 2;

/// Splits one BGP message off `r`: its type and its body (after the header).
/// The marker is not checked; BMP carries messages a router already accepted.
pub fn bgp_message<'a>(r: &mut Reader<'a>) -> Result<(u8, &'a [u8])> {
    let _marker = r.take(16, "bgp header truncated")?;
    let len = r.u16("bgp header truncated")? as usize;
    let kind = r.u8("bgp header truncated")?;
    let body_len = len
        .checked_sub(BGP_HEADER_LEN)
        .ok_or(WireError("bgp length below header size"))?;
    let body = r.take(body_len, "bgp message truncated")?;
    Ok((kind, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_never_reads_past_its_end() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert_eq!(r.u16("x"), Ok(0x0102));
        assert_eq!(r.u16("short"), Err(WireError("short")));
        assert_eq!(r.len(), 1, "a failed read consumes nothing");
        assert_eq!(r.u8("x"), Ok(3));
        assert!(r.is_empty());
        assert_eq!(r.take(1, "empty"), Err(WireError("empty")));
        assert_eq!(r.take(0, "empty"), Ok(&[][..]));
    }

    #[test]
    fn a_bgp_message_is_split_by_its_length() {
        let mut buf = vec![0xff; 16];
        buf.extend_from_slice(&(BGP_HEADER_LEN as u16 + 2).to_be_bytes());
        buf.push(BGP_UPDATE);
        buf.extend_from_slice(&[7, 8, 9]);
        let mut r = Reader::new(&buf);
        assert_eq!(bgp_message(&mut r), Ok((BGP_UPDATE, &[7, 8][..])));
        assert_eq!(r.rest(), &[9]);

        buf[16..18].copy_from_slice(&5u16.to_be_bytes());
        assert!(bgp_message(&mut Reader::new(&buf)).is_err());
        buf[16..18].copy_from_slice(&100u16.to_be_bytes());
        assert!(bgp_message(&mut Reader::new(&buf)).is_err());
    }
}
