// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Splits a BMP byte stream into messages.

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

#[cfg(test)]
mod tests {
    use super::*;
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
}
