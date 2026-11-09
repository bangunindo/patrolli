// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Splits a BMP or BGP byte stream into messages.

use super::wire::bgp::{
    BGP_KEEPALIVE, BGP_NOTIFICATION, BGP_ROUTE_REFRESH, MAX_EXTENDED_MESSAGE, MAX_MESSAGE, code,
    subcode,
};
use super::wire::bmp::{COMMON_HEADER_LEN, CommonHeader, VERSION};
use super::wire::{BGP_HEADER_LEN, BGP_OPEN, BGP_UPDATE};
use bytes::{Bytes, BytesMut};
use std::io;
use tokio_util::codec::Decoder;

/// The default cap on one BMP message. A Route Monitoring message is at most
/// one BGP message (65535 bytes with RFC 8654) plus its headers, so this
/// leaves room for BMPv4 TLVs and stops a bad length from buffering forever.
pub const DEFAULT_MAX_MESSAGE: usize = 1 << 20;

/// Why the stream cannot be read further. A BMP stream has no resync point,
/// so each of these ends the session.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("read failed: {0}")]
    Io(#[from] io::Error),
    #[error("unsupported bmp version {0}")]
    Version(u8),
    #[error("bmp message length {0} is out of bounds")]
    Length(u32),
}

impl FrameError {
    /// A constant label for counters and log fields.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Version(_) => "version",
            Self::Length(_) => "length",
        }
    }
}

/// One whole BMP message per item, common header included.
#[derive(Debug, Clone, Copy)]
pub struct BmpFrames {
    max: usize,
}

impl BmpFrames {
    pub fn new(max: usize) -> Self {
        Self {
            max: max.max(COMMON_HEADER_LEN),
        }
    }
}

impl Decoder for BmpFrames {
    type Item = Bytes;
    type Error = FrameError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Bytes>, FrameError> {
        let Some(head) = src.get(..COMMON_HEADER_LEN) else {
            return Ok(None);
        };
        let header = CommonHeader::parse(head).map_err(|_| FrameError::Length(0))?;
        if header.version != VERSION {
            return Err(FrameError::Version(header.version));
        }
        let len = header.length as usize;
        if len < COMMON_HEADER_LEN || len > self.max {
            return Err(FrameError::Length(header.length));
        }
        if src.len() < len {
            src.reserve(len - src.len());
            return Ok(None);
        }
        Ok(Some(src.split_to(len).freeze()))
    }
}

/// Why a BGP stream cannot be read further.
#[derive(Debug, thiserror::Error)]
pub enum BgpFrameError {
    #[error("read failed: {0}")]
    Io(#[from] io::Error),
    #[error("bgp marker is not all ones")]
    Marker,
    #[error("bgp message length {0} is out of bounds")]
    Length(u16),
    #[error("unknown bgp message type {0}")]
    Type(u8),
}

impl BgpFrameError {
    /// A constant label for counters and log fields.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Marker => "marker",
            Self::Length(_) => "length",
            Self::Type(_) => "bad message type",
        }
    }

    /// The NOTIFICATION that reports this error to the router, as (code,
    /// subcode, data); none for a failed read.
    pub fn notification(&self) -> Option<(u8, u8, Vec<u8>)> {
        match self {
            Self::Io(_) => None,
            Self::Marker => Some((
                code::MESSAGE_HEADER,
                subcode::CONNECTION_NOT_SYNCHRONIZED,
                Vec::new(),
            )),
            Self::Length(len) => Some((
                code::MESSAGE_HEADER,
                subcode::BAD_MESSAGE_LENGTH,
                len.to_be_bytes().to_vec(),
            )),
            Self::Type(kind) => {
                Some((code::MESSAGE_HEADER, subcode::BAD_MESSAGE_TYPE, vec![*kind]))
            }
        }
    }
}

/// The shortest OPEN, UPDATE and NOTIFICATION (RFC 4271 §4.2, §4.3, §4.5).
const MIN_OPEN: u16 = 29;
const MIN_UPDATE: u16 = 23;
const MIN_NOTIFICATION: u16 = 21;

/// One whole BGP message per item, header included. Messages are at most
/// 4096 bytes until extended messages are negotiated (RFC 8654).
#[derive(Debug, Clone, Copy)]
pub struct BgpFrames {
    max: usize,
}

impl Default for BgpFrames {
    fn default() -> Self {
        Self { max: MAX_MESSAGE }
    }
}

impl BgpFrames {
    /// Both sides advertised extended messages.
    pub fn allow_extended(&mut self) {
        self.max = MAX_EXTENDED_MESSAGE;
    }
}

impl Decoder for BgpFrames {
    type Item = Bytes;
    type Error = BgpFrameError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Bytes>, BgpFrameError> {
        let Some(head) = src.get(..BGP_HEADER_LEN) else {
            return Ok(None);
        };
        if head[..16].iter().any(|b| *b != 0xff) {
            return Err(BgpFrameError::Marker);
        }
        let len = u16::from_be_bytes([head[16], head[17]]);
        if (len as usize) < BGP_HEADER_LEN || len as usize > self.max {
            return Err(BgpFrameError::Length(len));
        }
        // RFC 4271 §6.1: each type's own length bounds, and only known types
        let fits = match head[18] {
            BGP_OPEN => len >= MIN_OPEN,
            BGP_UPDATE => len >= MIN_UPDATE,
            BGP_NOTIFICATION => len >= MIN_NOTIFICATION,
            BGP_KEEPALIVE => len as usize == BGP_HEADER_LEN,
            BGP_ROUTE_REFRESH => true,
            kind => return Err(BgpFrameError::Type(kind)),
        };
        if !fits {
            return Err(BgpFrameError::Length(len));
        }
        let len = len as usize;
        if src.len() < len {
            src.reserve(len - src.len());
            return Ok(None);
        }
        Ok(Some(src.split_to(len).freeze()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bgp::wire::bgp::{keepalive, message};
    use crate::bgp::wire::bmp::build::{initiation, termination};

    #[test]
    fn frames_split_on_their_length_however_the_bytes_arrive() {
        let mut stream = initiation("r1");
        stream.extend_from_slice(&termination(0));
        let mut codec = BmpFrames::new(DEFAULT_MAX_MESSAGE);
        let mut buf = BytesMut::new();
        let mut frames = Vec::new();
        for b in &stream {
            buf.extend_from_slice(&[*b]);
            while let Some(f) = codec.decode(&mut buf).unwrap() {
                frames.push(f);
            }
        }
        assert_eq!(frames.len(), 2);
        assert_eq!(&frames[0][..], &initiation("r1")[..]);
        assert!(buf.is_empty());
    }

    #[test]
    fn a_bad_header_ends_the_stream() {
        let mut codec = BmpFrames::new(64);
        let mut f = initiation("r1");
        f[0] = 1;
        assert!(matches!(
            codec.decode(&mut BytesMut::from(&f[..])),
            Err(FrameError::Version(1))
        ));
        for len in [0u32, 5, 65, u32::MAX] {
            let mut f = initiation("r1");
            f[1..5].copy_from_slice(&len.to_be_bytes());
            assert!(
                matches!(codec.decode(&mut BytesMut::from(&f[..])), Err(FrameError::Length(l)) if l == len),
                "{len}"
            );
        }
    }

    #[test]
    fn bgp_frames_check_marker_and_length() {
        let mut codec = BgpFrames::default();
        let mut buf = BytesMut::new();
        let mut stream = keepalive();
        stream.extend_from_slice(&message(2, &[0; 10]));
        for b in &stream {
            buf.extend_from_slice(&[*b]);
            if let Some(f) = codec.decode(&mut buf).unwrap() {
                assert!(f.len() == 19 || f.len() == 29);
            }
        }
        assert!(buf.is_empty());

        let mut bad = keepalive();
        bad[3] = 0;
        assert!(matches!(
            codec.decode(&mut BytesMut::from(&bad[..])),
            Err(BgpFrameError::Marker)
        ));
        for len in [0u16, 18, 4097] {
            let mut f = keepalive();
            f[16..18].copy_from_slice(&len.to_be_bytes());
            let err = codec.decode(&mut BytesMut::from(&f[..])).unwrap_err();
            assert!(matches!(err, BgpFrameError::Length(l) if l == len), "{len}");
            assert_eq!(err.notification(), Some((1, 2, len.to_be_bytes().to_vec())));
        }

        // RFC 4271 §6.1: each type's length bounds, Bad Message Length with
        // the length as data
        for (kind, len) in [(1u8, 28u16), (2, 22), (3, 20), (4, 20), (4, 23)] {
            let f = message(kind, &vec![0; len as usize - 19]);
            let err = codec.decode(&mut BytesMut::from(&f[..])).unwrap_err();
            assert!(
                matches!(err, BgpFrameError::Length(l) if l == len),
                "{kind} {len}"
            );
            assert_eq!(err.notification(), Some((1, 2, len.to_be_bytes().to_vec())));
        }
        for (kind, len) in [(1u8, 29usize), (2, 23), (3, 21), (4, 19), (5, 23)] {
            let f = message(kind, &vec![0; len - 19]);
            assert!(
                codec.decode(&mut BytesMut::from(&f[..])).unwrap().is_some(),
                "{kind}"
            );
        }
        // and Bad Message Type with the type
        for kind in [0u8, 6, 255] {
            let f = message(kind, &[]);
            let err = codec.decode(&mut BytesMut::from(&f[..])).unwrap_err();
            assert!(matches!(err, BgpFrameError::Type(k) if k == kind));
            assert_eq!(err.notification(), Some((1, 3, vec![kind])));
        }

        // 4097 bytes is fine once extended messages are on
        let big = message(2, &[0; 4097 - 19]);
        assert!(codec.decode(&mut BytesMut::from(&big[..])).is_err());
        codec.allow_extended();
        assert_eq!(
            codec
                .decode(&mut BytesMut::from(&big[..]))
                .unwrap()
                .unwrap()
                .len(),
            4097
        );
    }
}
