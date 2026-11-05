// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Splits a BMP or BGP byte stream into messages.

use super::wire::BGP_HEADER_LEN;
use super::wire::bgp::{MAX_EXTENDED_MESSAGE, MAX_MESSAGE, code, subcode};
use super::wire::bmp::{COMMON_HEADER_LEN, CommonHeader, VERSION};
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
}

impl BgpFrameError {
    /// A constant label for counters and log fields.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Marker => "marker",
            Self::Length(_) => "length",
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
        }
    }
}

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
    use crate::listener::bgp::wire::bgp::{keepalive, message};
    use crate::listener::bgp::wire::bmp::build::{initiation, termination};

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
