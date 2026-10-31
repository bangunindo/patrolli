// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! How a disk topic stores a message.
//!
//! A value is a 16-byte header, then the message's [`Codec`] encoding,
//! compressed as the header says:
//!
//! ```text
//! 0..4   MAGIC
//! 4      codec: 0 none, 1 lz4 (frame), 2 zstd
//! 5      flags: bit 0 transport optimized
//! 6..8   zero
//! 8..16  encoded length before compression, u64 LE
//! 16..   body
//! ```
//!
//! The message is written straight into the compressor, so no uncompressed
//! copy is held on the way in, and decompressed into one buffer of the
//! recorded length that the decoded Arrow arrays then point into. A value
//! names its own codec and layout, so a topic whose `compression` or
//! `transport_optimized` changed still reads what it stored before.

use crate::config::OtapCompression;
use anyhow::{Context, Result, bail, ensure};
use arrow::buffer::Buffer;
use bytes::Bytes;
use std::io::{Read, Write};
use std::sync::Arc;

const MAGIC: [u8; 4] = *b"ZCH\x01";
const HEADER: usize = 16;
const TRANSPORT_OPTIMIZED: u8 = 1;
/// zstd level for disk values.
const ZSTD_LEVEL: i32 = 3;
/// Most a decode reserves up front: a corrupt header must not allocate
/// whatever it claims.
const MAX_RESERVE: usize = 1 << 30;

/// A message a disk topic can store: an encoding that is cheap to write and
/// that decodes without copying out of the buffer it is read from.
pub trait Codec: Sized {
    /// Writes the message, in the OTAP transport encoding when
    /// `transport_optimized` (see `config::DiskChannel`). What it writes must
    /// keep the next byte 8-aligned relative to where it started, so Arrow
    /// data after it can be read in place.
    fn encode(&self, out: &mut dyn Write, transport_optimized: bool) -> Result<()>;

    /// About how many bytes [`encode`](Self::encode) writes, so an
    /// uncompressed value is allocated once instead of grown.
    fn size_hint(&self) -> usize {
        0
    }

    /// Reads back what [`encode`](Self::encode) wrote, with the same
    /// `transport_optimized`. `buf` holds exactly that and starts where the
    /// encoding started.
    fn decode(buf: Buffer, transport_optimized: bool) -> Result<Self>;
}

impl<T: Codec> Codec for Arc<T> {
    fn encode(&self, out: &mut dyn Write, transport_optimized: bool) -> Result<()> {
        T::encode(self, out, transport_optimized)
    }

    fn size_hint(&self) -> usize {
        T::size_hint(self)
    }

    fn decode(buf: Buffer, transport_optimized: bool) -> Result<Self> {
        T::decode(buf, transport_optimized).map(Arc::new)
    }
}

/// Counts what passes through, for the header's uncompressed length.
struct Counting<W> {
    inner: W,
    written: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub fn encode<M: Codec>(
    msg: &M,
    compression: OtapCompression,
    transport_optimized: bool,
) -> Result<Bytes> {
    let mut out = Vec::with_capacity(HEADER);
    out.extend_from_slice(&MAGIC);
    let flags = if transport_optimized {
        TRANSPORT_OPTIMIZED
    } else {
        0
    };
    // `OtapCompression`'s discriminant is the codec byte.
    out.extend_from_slice(&[compression as u8, flags, 0, 0]);
    out.extend_from_slice(&[0; 8]);
    let (mut out, len) = match compression {
        OtapCompression::None => {
            out.reserve(msg.size_hint());
            let mut w = Counting {
                inner: out,
                written: 0,
            };
            msg.encode(&mut w, transport_optimized)?;
            (w.inner, w.written)
        }
        OtapCompression::Lz4 => {
            let mut w = Counting {
                inner: lz4_flex::frame::FrameEncoder::new(out),
                written: 0,
            };
            msg.encode(&mut w, transport_optimized)?;
            let out = w.inner.finish().context("lz4 encode")?;
            (out, w.written)
        }
        OtapCompression::Zstd => {
            let mut w = Counting {
                inner: zstd::stream::Encoder::new(out, ZSTD_LEVEL)?,
                written: 0,
            };
            msg.encode(&mut w, transport_optimized)?;
            let out = w.inner.finish().context("zstd encode")?;
            (out, w.written)
        }
    };
    out[8..HEADER].copy_from_slice(&len.to_le_bytes());
    Ok(Bytes::from(out))
}

pub fn decode<M: Codec>(value: Bytes) -> Result<M> {
    ensure!(
        value.len() >= HEADER && value[..4] == MAGIC,
        "not a disk channel value"
    );
    let transport_optimized = value[5] & TRANSPORT_OPTIMIZED != 0;
    let len = usize::try_from(u64::from_le_bytes(value[8..HEADER].try_into()?))?;
    let body = value.slice(HEADER..);
    let buf = match value[4] {
        0 => {
            ensure!(
                body.len() == len,
                "value is {} bytes, header says {len}",
                body.len()
            );
            // In place: arrays point into the value. A misaligned one is
            // copied by the Arrow decoder.
            Buffer::from(body)
        }
        1 => {
            let mut out = Vec::with_capacity(len.min(MAX_RESERVE));
            lz4_flex::frame::FrameDecoder::new(&body[..])
                .read_to_end(&mut out)
                .context("lz4 decode")?;
            ensure!(
                out.len() == len,
                "decoded {} bytes, header says {len}",
                out.len()
            );
            Buffer::from_vec(out)
        }
        2 => {
            let mut out = Vec::with_capacity(len.min(MAX_RESERVE));
            zstd::stream::copy_decode(&body[..], &mut out).context("zstd decode")?;
            ensure!(
                out.len() == len,
                "decoded {} bytes, header says {len}",
                out.len()
            );
            Buffer::from_vec(out)
        }
        c => bail!("unknown codec {c}"),
    };
    M::decode(buf, transport_optimized)
}
