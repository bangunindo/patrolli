// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod cef;
mod cisco;
mod cise;
mod time;

use crate::chan::SenderChannel;
use crate::common::{ByteCounters, UdpListenerWorker, tls};
use crate::config::listener::ListenerKind;
use crate::listener::InternalMetrics;
use crate::payload::{LogRow, Scope, Value, ZString};
use crate::{cache, common, config, payload};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use bytes::{Buf, Bytes, BytesMut};
use flume::{Receiver, Sender};
use futures::FutureExt;
use nom::{
    IResult, Parser as NomParser,
    bytes::complete::{tag, take_while1},
};
use opentelemetry_proto::tonic::logs::v1::SeverityNumber;
use smol_str::SmolStr;
use socket2::{SockRef, TcpKeepalive};
use std::borrow::Cow;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use syslog_loose::ProcId;
use tokio::io::AsyncRead;
use tokio::net::TcpListener;
use tokio::sync::Mutex as TokioMutex;
use tokio::task::JoinSet;
use tokio::time::{MissedTickBehavior, interval, sleep};
use tokio_rustls::TlsAcceptor;
use tokio_stream::StreamExt;
use tokio_util::codec::{Decoder, Framed};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, Span, debug, error, info, warn};
use uuid::Uuid;

const STREAM_BUF_INITIAL: usize = 16 * 1024;
const MAX_OCTET_DIGITS: usize = 7;
const MAX_SYSLOG_MSG_SIZE: usize = 64 * 1024;
/// Capacity, in batches, of the channel from a TCP/TLS-only transport to the
/// workers. With UDP configured, `buffer_cap` sizes it instead.
const STREAM_CHANNEL_CAP: usize = 16;

type Packets = Vec<(Bytes, SocketAddr)>;

#[derive(thiserror::Error, Debug)]
enum SyslogFrameError {
    #[error("octet-count prefix exceeds {MAX_OCTET_DIGITS} digits")]
    OctetPrefixTooLong,
    #[error("invalid octet-count prefix")]
    OctetPrefixInvalid,
    #[error("message length {0} exceeds max {MAX_SYSLOG_MSG_SIZE}")]
    MessageTooLong(usize),
    #[error("unrecognized first byte: 0x{0:02x}")]
    UnknownFraming(u8),
}

impl From<SyslogFrameError> for std::io::Error {
    fn from(e: SyslogFrameError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
    }
}

/// Byte counts one connection has not yet added to the transport's
/// [`ByteCounters`], so decoding a frame touches no cache line shared with the
/// other connections.
#[derive(Default)]
struct ByteTally {
    total: u64,
    succeeded: u64,
    dropped: u64,
}

struct SyslogCodec {
    metrics: Arc<ByteCounters>,
    tally: ByteTally,
}

impl SyslogCodec {
    fn new(metrics: Arc<ByteCounters>) -> Self {
        Self {
            metrics,
            tally: ByteTally::default(),
        }
    }

    /// Adds what was decoded since the last flush to the shared counters.
    fn flush(&mut self) {
        let tally = std::mem::take(&mut self.tally);
        self.metrics
            .add(tally.total, tally.succeeded, tally.dropped);
    }
}

impl Drop for SyslogCodec {
    fn drop(&mut self) {
        self.flush();
    }
}

impl Decoder for SyslogCodec {
    type Item = Bytes;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> std::io::Result<Option<Bytes>> {
        let Some(&first) = src.first() else {
            return Ok(None);
        };
        match first {
            b'0'..=b'9' => decode_octet_counted(src, &mut self.tally),
            // `C` is raw CEF with no syslog header. CEF escapes newlines inside values, so a bare
            // LF always ends a record.
            b'<' | b'C' => decode_non_transparent(src, &mut self.tally),
            other => {
                self.tally.total += src.len() as u64;
                self.tally.dropped += src.len() as u64;
                src.clear();
                Err(SyslogFrameError::UnknownFraming(other).into())
            }
        }
    }
}

fn parse_octet_header(input: &[u8]) -> IResult<&[u8], usize> {
    let (rest, digits) = take_while1(|b: u8| b.is_ascii_digit()).parse(input)?;
    let (rest, _) = tag(b" ".as_ref()).parse(rest)?;
    // take_while1(is_ascii_digit) guarantees ASCII digits only.
    let s = std::str::from_utf8(digits).expect("ascii digits");
    let len: usize = s.parse().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit))
    })?;
    Ok((rest, len))
}

fn decode_octet_counted(
    src: &mut BytesMut,
    tally: &mut ByteTally,
) -> std::io::Result<Option<Bytes>> {
    // Bound the digit prefix scan so a flood of digits can't grow src unboundedly.
    let scan_end = src.len().min(MAX_OCTET_DIGITS + 1);
    let space_pos = src[..scan_end].iter().position(|&b| b == b' ');
    let header_len = match space_pos {
        Some(pos) => pos + 1,
        None if src.len() > MAX_OCTET_DIGITS => {
            tally.total += src.len() as u64;
            tally.dropped += src.len() as u64;
            src.clear();
            return Err(SyslogFrameError::OctetPrefixTooLong.into());
        }
        None => return Ok(None),
    };

    let msg_len = match parse_octet_header(&src[..header_len]) {
        Ok((_, len)) => len,
        Err(_) => {
            tally.total += src.len() as u64;
            tally.dropped += src.len() as u64;
            src.clear();
            return Err(SyslogFrameError::OctetPrefixInvalid.into());
        }
    };

    if msg_len == 0 || msg_len > MAX_SYSLOG_MSG_SIZE {
        tally.total += src.len() as u64;
        tally.dropped += src.len() as u64;
        src.clear();
        return Err(SyslogFrameError::MessageTooLong(msg_len).into());
    }

    let frame_len = header_len + msg_len;
    if src.len() < frame_len {
        src.reserve(frame_len - src.len());
        return Ok(None);
    }

    src.advance(header_len);
    let msg = src.split_to(msg_len).freeze();
    tally.total += frame_len as u64;
    tally.succeeded += frame_len as u64;
    Ok(Some(msg))
}

fn decode_non_transparent(
    src: &mut BytesMut,
    tally: &mut ByteTally,
) -> std::io::Result<Option<Bytes>> {
    let Some(pos) = src
        .iter()
        .position(|&b| b == b'\n' || b == b'\r' || b == b'\0')
    else {
        if src.len() > MAX_SYSLOG_MSG_SIZE {
            tally.total += src.len() as u64;
            tally.dropped += src.len() as u64;
            let dropped_len = src.len();
            src.clear();
            return Err(SyslogFrameError::MessageTooLong(dropped_len).into());
        }
        return Ok(None);
    };

    // `\r` may be the first half of `\r\n`; if the next byte hasn't arrived
    // yet, wait before committing.
    let delim_len = if src[pos] == b'\r' {
        match src.get(pos + 1) {
            None => return Ok(None),
            Some(&b'\n') => 2,
            Some(_) => 1,
        }
    } else {
        1
    };

    let msg = src.split_to(pos).freeze();
    src.advance(delim_len);
    let consumed = msg.len() + delim_len;
    tally.total += consumed as u64;
    tally.succeeded += consumed as u64;
    Ok(Some(msg))
}

/// Finds and parses a CEF record in `packet_str`, returning its byte offset. syslog_loose reads
/// the start of an RFC3164 CEF record as header fields (`CEF:0|...` as the hostname, or `CEF` as
/// the tag), so any header field that lies inside the CEF record is cleared.
fn apply_cef<'a>(
    packet_str: &'a str,
    msg: &mut syslog_loose::Message<&'a str>,
) -> Option<(usize, cef::Cef<'a>)> {
    let off = cef::find(packet_str)?;
    let parsed = cef::parse(&packet_str[off..]).ok()?;
    clear_fields_from(packet_str, msg, off);
    Some((off, parsed))
}

/// Clears the header fields syslog_loose read from at or after byte `off`, the start of a record
/// it does not know: those are pieces of the record, not header.
fn clear_fields_from(packet_str: &str, msg: &mut syslog_loose::Message<&str>, off: usize) {
    let base = packet_str.as_ptr() as usize;
    let inside = |field: &str| {
        let start = (field.as_ptr() as usize).wrapping_sub(base);
        start >= off && start <= packet_str.len()
    };
    if msg.hostname.is_some_and(inside) {
        msg.hostname = None;
    }
    if msg.appname.is_some_and(inside) {
        msg.appname = None;
        msg.procid = None;
    }
    if msg.msgid.is_some_and(inside) {
        msg.msgid = None;
    }
}

/// When neither RFC parses, syslog_loose makes the whole packet the message, PRI included. The
/// PRI is still readable then, as on most Cisco IOS headers (`<189>521: rtr-lab-01: …`).
fn recover_pri<'a>(packet_str: &'a str, msg: &mut syslog_loose::Message<&'a str>) {
    let fell_back = msg.facility.is_none()
        && msg.timestamp.is_none()
        && msg.hostname.is_none()
        && std::ptr::eq(msg.msg, packet_str);
    if !fell_back {
        return;
    }
    let Some((pri, rest)) = packet_str.strip_prefix('<').and_then(|s| s.split_once('>')) else {
        return;
    };
    if !(1..=3).contains(&pri.len()) || !pri.bytes().all(|b| b.is_ascii_digit()) {
        return;
    }
    let Some(pri) = pri.parse::<u8>().ok().filter(|p| *p <= 191) else {
        return;
    };
    (msg.facility, msg.severity) = syslog_loose::decompose_pri(pri);
    msg.msg = rest;
}

/// Finds a Cisco `%FACILITY-SEVERITY-MNEMONIC:` tag and reads the device preamble before it,
/// filling the hostname from it when the header had none.
fn apply_cisco<'a>(
    packet_str: &'a str,
    msg: &mut syslog_loose::Message<&'a str>,
) -> Option<(cisco::Tag<'a>, cisco::Preamble<'a>)> {
    let tag = cisco::find(packet_str)?;
    clear_fields_from(packet_str, msg, tag.at);
    let msg_at = (msg.msg.as_ptr() as usize).wrapping_sub(packet_str.as_ptr() as usize);
    let pre = if msg_at <= tag.at && tag.at <= msg_at + msg.msg.len() {
        cisco::preamble(&packet_str[msg_at..tag.at])
    } else {
        cisco::Preamble::default()
    };
    if msg.hostname.is_none() {
        msg.hostname = pre.host;
    }
    Some((tag, pre))
}

/// Finds a Cisco ISE/ACS record: a `CISE_`/`CSCOacs_` tag followed by the segment counters,
/// returning the category and the segment. With no hostname in the header, syslog_loose reads
/// the tag as the hostname and the `msg_id` as the tag, so those fields are put right and `msg`
/// restarts at the `msg_id`.
fn apply_cise<'a>(
    packet_str: &'a str,
    msg: &mut syslog_loose::Message<&'a str>,
) -> Option<(&'a str, cise::Segment<'a>)> {
    if let Some(tag) = msg.appname
        && msg.procid.is_none()
        && let Some(category) = cise::category(tag)
    {
        return Some((category, cise::segment(msg.msg)?));
    }
    let tag = msg.hostname?;
    let category = cise::category(tag)?;
    let after_tag = (tag.as_ptr() as usize - packet_str.as_ptr() as usize) + tag.len();
    let seg = cise::segment(&packet_str[after_tag..])?;
    msg.hostname = None;
    msg.appname = Some(tag);
    msg.procid = None;
    msg.msgid = None;
    msg.msg = seg.record;
    Some((category, seg))
}

/// What a syslog message carries beyond its header.
enum Format<'a> {
    Plain,
    /// The record's byte offset in the packet, and the record.
    Cef(usize, cef::Cef<'a>),
    /// The category and the segment.
    Cise(&'a str, cise::Segment<'a>),
    Cisco(cisco::Tag<'a>, cisco::Preamble<'a>),
}

/// The payload formats a listener looks for, each switched by its config section.
#[derive(Clone, Copy)]
struct Detect {
    cef: bool,
    cise: bool,
    cisco: bool,
}

impl Detect {
    #[cfg(test)]
    const ALL: Self = Self {
        cef: true,
        cise: true,
        cisco: true,
    };

    fn new(conf: &config::listener::Syslog) -> Self {
        Self {
            cef: conf.cef.enabled,
            cise: conf.cise.enabled,
            cisco: conf.cisco.enabled,
        }
    }
}

fn parse_message(packet_str: &str, detect: Detect) -> (syslog_loose::Message<&str>, Format<'_>) {
    let mut msg = syslog_loose::parse_message(packet_str, syslog_loose::Variant::Either);
    recover_pri(packet_str, &mut msg);
    let format = if detect.cef
        && let Some((off, cef)) = apply_cef(packet_str, &mut msg)
    {
        Format::Cef(off, cef)
    } else if detect.cise
        && let Some((category, seg)) = apply_cise(packet_str, &mut msg)
    {
        Format::Cise(category, seg)
    } else if detect.cisco
        && let Some((tag, pre)) = apply_cisco(packet_str, &mut msg)
    {
        Format::Cisco(tag, pre)
    } else {
        Format::Plain
    };
    (msg, format)
}

/// What the transport is doing. Workers start it; only `stop` ends it.
enum State {
    Idle,
    Running(Running),
    /// Stopped by the listener's close; never started again.
    Closed,
}

/// A started transport. Every producer (UDP reader threads, TCP/TLS servers
/// and their streams) holds a sender of `rx`'s channel and nothing else does,
/// so the channel disconnects once they have all handed over and returned.
/// Dropping it aborts every task it runs.
struct Running {
    rx: Receiver<Packets>,
    /// Stops the intake. A server that returns on its own cancels it too,
    /// bringing every producer down with it so the transport restarts whole.
    intake: CancellationToken,
    udp: Option<UdpListenerWorker>,
    /// The TCP and TLS servers, each owning its streams.
    servers: JoinSet<()>,
}

impl Running {
    /// Stops the producers and waits until each has sent what it read.
    async fn shutdown(mut self) {
        self.intake.cancel();
        // Only the workers' receivers are left, so with none running (scaled
        // to zero) the readers' last sends fail at once instead of waiting
        // out the drain timeout.
        drop(self.rx);
        if let Some(udp) = self.udp.take() {
            udp.close().await;
        }
        while let Some(res) = self.servers.join_next().await {
            if let Err(e) = res {
                error!(error = %e, "server task failed");
            }
        }
    }
}

struct SyslogTransport {
    id: Uuid,
    name: String,
    conf: config::listener::SyslogListener,
    batch: config::Batch,
    /// Every producer's bytes, UDP readers included; kept across restarts.
    internal_metrics: Arc<ByteCounters>,
    state: TokioMutex<State>,
}

impl SyslogTransport {
    fn new(
        id: Uuid,
        name: String,
        conf: config::listener::SyslogListener,
        batch: config::Batch,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            name,
            conf,
            batch,
            internal_metrics: Arc::default(),
            state: TokioMutex::new(State::Idle),
        })
    }

    /// How long the transport holds a partial batch. `batch_timeout` is split
    /// evenly between the transport and the worker, which merges the small
    /// batches of quiet sources into full payloads, so a record waits at most
    /// `batch_timeout` in all.
    fn linger(&self) -> Duration {
        self.batch.batch_timeout / 2
    }

    /// Returns the channel the workers read, starting the transport unless it
    /// runs. Every worker calls this, so a worker restarted after a failed
    /// start retries it, and one restarted after the transport died starts it
    /// afresh. `None` once the listener is closed.
    async fn ensure_started(self: &Arc<Self>) -> Result<Option<Receiver<Packets>>> {
        let mut state = self.state.lock().await;
        match &*state {
            State::Running(running) if !running.intake.is_cancelled() => {
                return Ok(Some(running.rx.clone()));
            }
            State::Closed => return Ok(None),
            State::Running(_) | State::Idle => {}
        }
        if let State::Running(dead) = std::mem::replace(&mut *state, State::Idle) {
            warn!("transport stopped unexpectedly, restarting");
            dead.shutdown().await;
        }
        let running = self.clone().start().await?;
        let rx = running.rx.clone();
        *state = State::Running(running);
        Ok(Some(rx))
    }

    /// Binds every configured transport. On error, what was already started
    /// is dropped with `running`, which stops it.
    async fn start(self: Arc<Self>) -> Result<Running> {
        debug!("transport starting");
        let cap = self
            .conf
            .udp
            .as_ref()
            .map_or(STREAM_CHANNEL_CAP, |udp| udp.buffer_cap);
        let (tx, rx) = flume::bounded(cap);
        let mut running = Running {
            rx,
            intake: CancellationToken::new(),
            udp: None,
            servers: JoinSet::new(),
        };

        if let Some(tcp) = &self.conf.tcp {
            let listener = TcpListener::bind(tcp.address)
                .await
                .with_context(|| format!("bind tcp {}", tcp.address))?;
            info!(transport = "tcp", addr = %tcp.address, "server listening");
            let filter = IpFilter::new(&tcp.ip_whitelist, &tcp.ip_blacklist);
            running.servers.spawn(
                self.clone()
                    .server(listener, None, filter, tx.clone(), running.intake.clone())
                    .instrument(Span::current()),
            );
        }

        if let Some(tls) = &self.conf.tls {
            let server_config =
                tls::build_rustls_server_config(&tls.tls).context("build tls server config")?;
            let acceptor = TlsAcceptor::from(Arc::new(server_config));
            let listener = TcpListener::bind(tls.address)
                .await
                .with_context(|| format!("bind tls {}", tls.address))?;
            info!(transport = "tls", addr = %tls.address, "server listening");
            let filter = IpFilter::new(&tls.ip_whitelist, &tls.ip_blacklist);
            running.servers.spawn(
                self.clone()
                    .server(
                        listener,
                        Some(acceptor),
                        filter,
                        tx.clone(),
                        running.intake.clone(),
                    )
                    .instrument(Span::current()),
            );
        }

        if let Some(udp) = &self.conf.udp {
            running.udp = Some(UdpListenerWorker::new(
                (self.id, self.name.clone()),
                udp.address,
                tx,
                &running.intake,
                self.internal_metrics.clone(),
                udp.worker_count,
                udp.max_packet_size,
                udp.recv_buffer_size,
                Duration::from_micros(udp.spin_before_block_us),
                self.batch.batch_size,
                self.linger(),
                udp.ip_whitelist.clone(),
                udp.ip_blacklist.clone(),
            )?);
        }

        Ok(running)
    }

    /// Stops the transport for good: intake first, then the producers send
    /// what they read and return, which disconnects the workers' channel. The
    /// workers must keep receiving meanwhile; the caller bounds the wait.
    async fn stop(&self) {
        let state = std::mem::replace(&mut *self.state.lock().await, State::Closed);
        if let State::Running(running) = state {
            running.shutdown().await;
            debug!("transport stopped");
        }
    }

    /// Accepts TCP (or, with `acceptor`, TLS) connections until `intake` is
    /// cancelled, then waits for its streams to send what they read.
    async fn server(
        self: Arc<Self>,
        listener: TcpListener,
        acceptor: Option<TlsAcceptor>,
        filter: IpFilter,
        tx: Sender<Packets>,
        intake: CancellationToken,
    ) {
        let transport = if acceptor.is_some() { "tls" } else { "tcp" };
        // However this returns, the rest of the intake goes down with it.
        let _intake_guard = intake.clone().drop_guard();
        let mut streams = JoinSet::new();
        loop {
            while streams.try_join_next().is_some() {}
            let (stream, addr) = tokio::select! {
                biased;

                _ = intake.cancelled() => break,
                accepted = listener.accept() => match accepted {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(transport, error = %e, "accept failed");
                        continue;
                    }
                },
            };
            if !filter.allows(&addr) {
                debug!(peer = %addr, "connection rejected by ip filter");
                continue;
            }
            let ka = TcpKeepalive::new()
                .with_time(Duration::from_secs(60))
                .with_interval(Duration::from_secs(20))
                .with_retries(4);
            if let Err(e) = SockRef::from(&stream).set_tcp_keepalive(&ka) {
                debug!(peer = %addr, error = %e, "set tcp keepalive failed");
            };

            let (this, tx, intake, acceptor) =
                (self.clone(), tx.clone(), intake.clone(), acceptor.clone());
            streams.spawn(
                async move {
                    let Some(acceptor) = acceptor else {
                        return this.handle_syslog_stream(intake, tx, stream, addr).await;
                    };
                    // A client stalling its handshake must not hold shutdown.
                    let accepted = tokio::select! {
                        _ = intake.cancelled() => return,
                        accepted = acceptor.accept(stream) => accepted,
                    };
                    match accepted {
                        Ok(stream) => this.handle_syslog_stream(intake, tx, stream, addr).await,
                        Err(e) => warn!(peer = %addr, error = %e, "tls handshake failed"),
                    }
                }
                .instrument(Span::current()),
            );
        }
        // Free the port before waiting on the streams.
        drop(listener);
        drop(tx);
        while let Some(res) = streams.join_next().await {
            if let Err(e) = res {
                error!(transport, error = %e, "stream task failed");
            }
        }
        info!(transport, "server stopped");
    }

    /// Reads one connection until `intake` is cancelled or the peer is done.
    /// Frames are batched per stream: a batch goes out once it holds
    /// `batch_size` frames or its first frame is [`linger`](Self::linger) old, so the
    /// channel sees one operation per batch however busy or quiet the peer
    /// is. A send waits for the workers, which slows down this peer only.
    async fn handle_syslog_stream<R>(
        &self,
        intake: CancellationToken,
        tx: Sender<Packets>,
        stream: R,
        addr: SocketAddr,
    ) where
        R: AsyncRead + Unpin,
    {
        let codec = SyslogCodec::new(self.internal_metrics.clone());
        let mut framed = Framed::with_capacity(stream, codec, STREAM_BUF_INITIAL);
        let batch_size = self.batch.batch_size;
        let linger = self.linger();
        let idle = self.conf.idle_timeout;
        let mut pending: Packets = Vec::new();
        // Both timers live as long as the stream and are re-armed, never
        // recreated: a quiet peer would otherwise register one per frame.
        // `flush` fires once the oldest pending frame has waited long enough.
        let flush = sleep(Duration::ZERO);
        tokio::pin!(flush);
        let mut flush_armed = false;
        // `idle_check` fires at the earliest the peer can have been quiet for
        // `idle`; it is pushed back only then, not on every frame.
        let mut last_frame = tokio::time::Instant::now();
        let idle_check = sleep(idle);
        tokio::pin!(idle_check);
        let reason = loop {
            let frame = tokio::select! {
                biased;

                _ = intake.cancelled() => break "cancelled",
                () = &mut flush, if flush_armed => {
                    flush_armed = false;
                    if !send_pending(&tx, &mut pending).await {
                        break "closed";
                    }
                    continue;
                }
                frame = framed.next() => frame,
                () = &mut idle_check, if !idle.is_zero() => {
                    let quiet_until = last_frame + idle;
                    if tokio::time::Instant::now() >= quiet_until {
                        break "idle_timeout";
                    }
                    idle_check.as_mut().reset(quiet_until);
                    continue;
                }
            };
            last_frame = tokio::time::Instant::now();
            // Take what is already readable without going back to `select!`.
            let mut ended = take_frame(frame, &mut pending, addr);
            while ended.is_none() && pending.len() < batch_size {
                match framed.next().now_or_never() {
                    Some(frame) => ended = take_frame(frame, &mut pending, addr),
                    None => break,
                }
            }
            // Once per wake-up rather than per frame.
            framed.codec_mut().flush();
            if pending.len() >= batch_size {
                flush_armed = false;
                if !send_pending(&tx, &mut pending).await {
                    break "closed";
                }
            } else if !flush_armed && !pending.is_empty() {
                flush.as_mut().reset(last_frame + linger);
                flush_armed = true;
            }
            if let Some(reason) = ended {
                break reason;
            }
        };
        if reason != "closed" {
            send_pending(&tx, &mut pending).await;
        }
        debug!(peer = %addr, reason, "stream closed");
    }

    fn metrics(&self) -> common::ByteCounts {
        self.internal_metrics.get()
    }
}

/// Sends `pending` unless it is empty. False once the workers are gone.
async fn send_pending(tx: &Sender<Packets>, pending: &mut Packets) -> bool {
    if pending.is_empty() {
        return true;
    }
    // Sized after this batch: a busy stream keeps full-size buffers, a quiet
    // one (of thousands, maybe) small ones.
    let next = Vec::with_capacity(pending.len());
    tx.send_async(std::mem::replace(pending, next))
        .await
        .is_ok()
}

/// Adds a decoded frame to `pending`, or returns why the stream ended.
fn take_frame(
    frame: Option<std::io::Result<Bytes>>,
    pending: &mut Packets,
    addr: SocketAddr,
) -> Option<&'static str> {
    match frame {
        Some(Ok(msg)) => {
            pending.push((msg, addr));
            None
        }
        // framing or I/O error
        Some(Err(e)) => {
            debug!(peer = %addr, error = %e, "stream read failed");
            Some("read_error")
        }
        None => Some("eof"),
    }
}

/// A server's connection allow/deny lists.
struct IpFilter {
    whitelist: Vec<IpAddr>,
    blacklist: Vec<IpAddr>,
}

impl IpFilter {
    fn new(whitelist: &[IpAddr], blacklist: &[IpAddr]) -> Self {
        Self {
            whitelist: whitelist.to_vec(),
            blacklist: blacklist.to_vec(),
        }
    }

    fn allows(&self, addr: &SocketAddr) -> bool {
        common::ip_allowed(&addr.ip(), &self.whitelist, &self.blacklist)
    }
}

pub struct SyslogListener {
    conf: config::listener::Listener,
    channel: SenderChannel<&'static str, payload::ProcessorPayload>,
    forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
    conf_cache: cache::Cache,
    node_id: Option<Uuid>,

    internal_metrics: Arc<InternalMetrics>,
    transport: Arc<SyslogTransport>,
    cise: config::listener::Cise,
    detect: Detect,
    /// Shared by the workers, which the segments of one ISE event can reach
    /// separately.
    reassembler: parking_lot::Mutex<cise::Reassembler>,
}

impl SyslogListener {
    pub fn new(
        conf: config::listener::Listener,
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
    ) -> Result<Self> {
        let config::listener::Config::Syslog(syslog_config) = &conf.config else {
            return Err(anyhow!("listener config is not syslog"));
        };
        let internal_metrics = Arc::new(InternalMetrics::default());
        let cise = syslog_config.cise.clone();
        let detect = Detect::new(syslog_config);
        let reassembler = parking_lot::Mutex::new(cise::Reassembler::new(&cise));
        let transport = SyslogTransport::new(
            conf.id,
            conf.name.to_string(),
            syslog_config.listener.clone(),
            conf.batch,
        );
        Ok(Self {
            conf,
            channel,
            forwarder,
            conf_cache,
            node_id,
            internal_metrics,
            transport,
            cise,
            detect,
            reassembler,
        })
    }

    fn parse_packets(&self, batch: &mut payload::LogsBuilder, packets: Vec<(Bytes, SocketAddr)>) {
        self.internal_metrics
            .total_items
            .fetch_add(packets.len() as u64, Ordering::Relaxed);
        let mut success_count = 0usize;
        let mut records = 0usize;
        let mut assembled = Vec::new();
        for (packet, addr) in &packets {
            let packet_str: &str = match str::from_utf8(packet) {
                Ok(v) => v,
                Err(_) => {
                    self.internal_metrics
                        .total_bytes_errored
                        .fetch_add(packet.len() as u64, Ordering::Relaxed);
                    self.internal_metrics
                        .total_items_errored
                        .fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };
            success_count += 1;
            let (msg, format) = parse_message(packet_str, self.detect);
            if let Format::Cise(_, seg) = &format
                && (2..=self.cise.max_segments).contains(&seg.total)
            {
                let key = (
                    addr.ip(),
                    SmolStr::new(msg.hostname.unwrap_or_default()),
                    seg.msg_id,
                );
                self.reassembler.lock().offer(
                    key,
                    *addr,
                    seg,
                    packet_str,
                    Instant::now(),
                    &mut assembled,
                );
                continue;
            }
            self.push_record(batch, packet_str, *addr, msg, format, 1);
            records += 1;
        }
        records += self.push_assembled(batch, assembled);
        self.internal_metrics
            .total_items_succeeded
            .fetch_add(success_count as u64, Ordering::Relaxed);
        // A segment of a split ISE event produces none until the event is put back together.
        self.internal_metrics
            .total_records
            .fetch_add(records as u64, Ordering::Relaxed);
    }

    /// Adds the ISE events the reassembler is done with. Returns how many.
    fn push_assembled(
        &self,
        batch: &mut payload::LogsBuilder,
        assembled: Vec<cise::Assembled>,
    ) -> usize {
        let n = assembled.len();
        for a in assembled {
            let (msg, format) = parse_message(&a.text, self.detect);
            self.push_record(batch, &a.text, a.addr, msg, format, a.received);
        }
        n
    }

    /// Adds the ISE events the reassembler gives up on: those past the
    /// reassembly timeout, or, with `all`, every one held.
    fn flush_reassembler(&self, batch: &mut payload::LogsBuilder, all: bool) {
        let mut assembled = Vec::new();
        {
            // Another worker sweeping does it for this one too.
            let reassembler = if all {
                Some(self.reassembler.lock())
            } else {
                self.reassembler.try_lock()
            };
            let Some(mut reassembler) = reassembler else {
                return;
            };
            if all {
                reassembler.drain(&mut assembled);
            } else if !reassembler.is_empty() {
                reassembler.expire(Instant::now(), &mut assembled);
            }
        }
        if assembled.is_empty() {
            return;
        }
        let records = self.push_assembled(batch, assembled);
        self.internal_metrics
            .total_records
            .fetch_add(records as u64, Ordering::Relaxed);
    }

    /// Adds one record. `received` is how many segments of its ISE event the
    /// message holds.
    fn push_record(
        &self,
        batch: &mut payload::LogsBuilder,
        packet_str: &str,
        addr: SocketAddr,
        msg: syslog_loose::Message<&str>,
        format: Format<'_>,
        received: u32,
    ) {
        let mut attrs = smallvec::SmallVec::<[(ZString, Value); 16]>::new();
        let mut log_record = LogRow {
            source_ip: Some(addr.ip()),
            source_port: Some(addr.port()),
            ..Default::default()
        };
        // should we include protocl as well?
        let protocol_string = match (&format, msg.protocol) {
            (Format::Cef(0, _), _) => "CEF",
            (_, syslog_loose::Protocol::RFC3164) => "RFC3164",
            (_, syslog_loose::Protocol::RFC5424(_)) => "RFC5424",
        };
        attrs.push((
            ZString::from_static("protocol"),
            Value::String(ZString::from_ref(protocol_string)),
        ));
        // What the payload was recognised as, named after the attribute that
        // holds its fields. `protocol` stays the header.
        let payload_format = match &format {
            Format::Plain => None,
            Format::Cef(..) => Some("cef"),
            Format::Cise(..) => Some("cise"),
            Format::Cisco(..) => Some("cisco"),
        };
        if let Some(payload_format) = payload_format {
            attrs.push((
                ZString::from_static("format"),
                Value::String(ZString::from_static(payload_format)),
            ));
        }
        if let Some(facility) = msg.facility {
            attrs.push((
                ZString::from_static("facility_level"),
                Value::String(ZString::from_ref(facility.as_str())),
            ));
        }
        if let Some(severity) = msg.severity {
            attrs.push((
                ZString::from_static("severity_level"),
                Value::String(ZString::from_ref(severity.as_str())),
            ));

            // convert from syslog severity to LogSeverity
            log_record.severity = Some(match severity {
                syslog_loose::SyslogSeverity::SEV_EMERG => SeverityNumber::Fatal3,
                syslog_loose::SyslogSeverity::SEV_ALERT => SeverityNumber::Fatal2,
                syslog_loose::SyslogSeverity::SEV_CRIT => SeverityNumber::Fatal,
                syslog_loose::SyslogSeverity::SEV_ERR => SeverityNumber::Error,
                syslog_loose::SyslogSeverity::SEV_WARNING => SeverityNumber::Warn,
                syslog_loose::SyslogSeverity::SEV_NOTICE => SeverityNumber::Info2,
                syslog_loose::SyslogSeverity::SEV_INFO => SeverityNumber::Info,
                syslog_loose::SyslogSeverity::SEV_DEBUG => SeverityNumber::Debug,
            });
        }
        if let Some(timestamp) = msg.timestamp {
            log_record.observed_time = Some(timestamp.into());
        }
        if let Some(hostname) = msg.hostname {
            attrs.push((
                ZString::from_static("hostname"),
                Value::String(ZString::from_ref(hostname)),
            ));
        }
        if let Some(appname) = msg.appname {
            attrs.push((
                ZString::from_static("appname"),
                Value::String(ZString::from_ref(appname)),
            ));
        }
        if let Some(procid) = msg.procid {
            attrs.push((
                ZString::from_static("procid"),
                match procid {
                    ProcId::PID(num) => Value::Int64(num as i64),
                    ProcId::Name(name) => Value::String(ZString::from_ref(name)),
                },
            ));
        }
        if let Some(msgid) = msg.msgid {
            attrs.push((
                ZString::from_static("msgid"),
                Value::String(ZString::from_ref(msgid)),
            ));
        }
        let structured_data_map = msg
            .structured_data
            .into_iter()
            .map(|elem| {
                let mut fields: Vec<Value> = Vec::with_capacity(elem.params.len());
                for (k, v) in elem.params {
                    let kv = vec![
                        (
                            ZString::from_ref("key"),
                            Value::String(ZString::from_ref(k)),
                        ),
                        (
                            ZString::from_ref("value"),
                            Value::String(ZString::from_ref(v)),
                        ),
                    ];
                    fields.push(Value::KvList(kv));
                }

                let result = vec![
                    (
                        ZString::from_ref("id"),
                        Value::String(ZString::from_ref(elem.id)),
                    ),
                    (ZString::from_ref("params"), Value::Array(fields)),
                ];

                Value::KvList(result)
            })
            .collect::<Vec<_>>();
        attrs.push((
            ZString::from_static("data"),
            Value::Array(structured_data_map),
        ));
        log_record.body = Some(Cow::Borrowed(msg.msg));
        match format {
            Format::Plain => {}
            Format::Cef(off, cef) => {
                let cef = cef.into_attrs();
                if cef.severity.is_some() {
                    log_record.severity = cef.severity;
                }
                if cef.rt.is_some() {
                    log_record.observed_time = cef.rt;
                }
                // RFC3164 drops the `CEF:` prefix from `msg`, so take the record from the packet.
                log_record.body = Some(Cow::Borrowed(packet_str[off..].trim_end()));
                attrs.push((ZString::from_static("cef"), cef.value));
            }
            Format::Cise(category, seg) => {
                let cise = cise::parse(seg.body).into_attrs(category, &seg, received);
                if cise.severity.is_some() {
                    log_record.severity = cise.severity;
                }
                if cise.time.is_some() {
                    log_record.observed_time = cise.time;
                }
                log_record.body = Some(Cow::Borrowed(seg.record));
                attrs.push((ZString::from_static("cise"), cise.value));
            }
            Format::Cisco(tag, pre) => {
                let cisco = cisco::into_attrs(&tag, &pre);
                if cisco.severity.is_some() {
                    log_record.severity = cisco.severity;
                }
                if cisco.time.is_some() {
                    log_record.observed_time = cisco.time;
                }
                log_record.body = Some(Cow::Borrowed(packet_str[tag.at..].trim_end()));
                attrs.push((ZString::from_static("cisco"), cisco.value));
            }
        }

        log_record.attrs = &attrs;
        batch.push_row(log_record);
    }

    fn set_scope(&self, batch: &mut payload::LogsBuilder) {
        batch.scope(Scope {
            name: self.conf.name.clone(),
            listener_id: Some(self.conf.id),
            listener_type: Some(SmolStr::new_static(
                ListenerKind::from(&self.conf.config).into(),
            )),
            listener_name: Some(self.conf.name.clone()),
            monitor_type: None,
            node_id: config::NODE_ID.get().copied(),
        });
    }

    /// Parses one batch from the transport into `batch`, handing `batch` to
    /// the pipeline once it is full, or first if the two together would pass
    /// `batch_size`. True when it sent.
    async fn handle_packets(&self, batch: &mut payload::LogsBuilder, packets: Packets) -> bool {
        let batch_size = self.transport.batch.batch_size;
        let mut sent = false;
        if batch.num_items() + packets.len() > batch_size {
            self.send_batch(batch).await;
            sent = true;
        }
        let n = Instant::now();
        self.parse_packets(batch, packets);
        self.internal_metrics
            .engine_time_spent
            .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        if batch.num_items() >= batch_size {
            self.send_batch(batch).await;
            sent = true;
        }
        sent
    }

    async fn send_batch(&self, batch: &mut payload::LogsBuilder) {
        if batch.is_empty() {
            return;
        }
        let pl = batch.finish();
        super::send_batch(super::SendBatch {
            batch: pl,
            conf: &self.conf,
            conf_cache: &self.conf_cache,
            node_id: self.node_id,
            internal_metrics: &self.internal_metrics,
            channel: &self.channel,
            forwarder: &self.forwarder,
        })
        .await;
        // `finish` resets the builder (clearing the scope), so re-arm the scope
        // before the next batch starts collecting rows.
        self.set_scope(batch);
    }
}

#[async_trait]
impl super::Listener for SyslogListener {
    #[tracing::instrument(skip_all, fields(listener_id = %self.conf.id, listener = %self.conf.name))]
    async fn listen(&self, token: CancellationToken) {
        let recv = match self.transport.ensure_started().await {
            Ok(Some(recv)) => recv,
            // Closed meanwhile.
            Ok(None) => return,
            Err(e) => {
                error!(error = %format!("{e:#}"), "transport failed to start");
                return;
            }
        };
        self.internal_metrics
            .total_workers
            .fetch_add(1, Ordering::Relaxed);
        info!("starting listener");
        let mut batch = payload::LogsBuilder::new();
        self.set_scope(&mut batch);
        // The worker's half of `batch_timeout`; see `SyslogTransport::linger`.
        let mut tick = interval(self.transport.linger());
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            let n = Instant::now();
            tokio::select! {
                biased;

                _ = token.cancelled() => {
                    debug!("stopping listener");
                    break;
                }
                _ = tick.tick() => {
                    self.internal_metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    self.flush_reassembler(&mut batch, false);
                    self.send_batch(&mut batch).await;
                }
                res = recv.recv_async() => {
                    self.internal_metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    // Disconnected: the transport stopped, or died and the
                    // supervisor restarts this worker.
                    let Ok(packets) = res else {
                        debug!("transport closed");
                        break;
                    };
                    if self.handle_packets(&mut batch, packets).await {
                        tick.reset();
                    }
                }
            }
            self.internal_metrics
                .total_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        // Once the transport has stopped, what is queued is all that is left,
        // and no other worker may be around to take it.
        if recv.is_disconnected() {
            for packets in recv.drain() {
                self.handle_packets(&mut batch, packets).await;
            }
            // Nothing more of the held ISE events can arrive.
            self.flush_reassembler(&mut batch, true);
        }
        self.send_batch(&mut batch).await;
        info!("listener stopped");
        self.internal_metrics
            .total_workers
            .fetch_sub(1, Ordering::Relaxed);
    }

    async fn stop(&self) {
        self.transport.stop().await;
    }

    async fn metrics(&self) -> common::internal_metrics::Listener {
        let mut worker_metrics = self
            .internal_metrics
            .get(self.conf.id, self.conf.name.to_string());
        let bytes_metrics = self.transport.metrics();
        worker_metrics.total_bytes += bytes_metrics.total_bytes;
        worker_metrics.total_bytes_succeeded += bytes_metrics.total_bytes_succeeded;
        worker_metrics.total_bytes_dropped += bytes_metrics.total_bytes_dropped;
        worker_metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closing a listener whose pipeline is backed up: the shape of a restart
    /// under traffic. Two workers, a processor channel holding one payload
    /// that nobody reads until the close starts, and small batches, so every
    /// stage of the transport is holding data when the workers are cancelled.
    mod close {
        use super::*;
        use crate::listener::DRAIN_TIMEOUT;
        use crate::listener::test_util::{self, CLOSE_LIMIT, Port, item_for};
        use std::io::Write as _;

        const MESSAGES: usize = 1000;

        /// Fixed length, so a byte count converts to a message count.
        fn messages() -> Vec<String> {
            (0..MESSAGES)
                .map(|i| format!("<13>1 2026-10-02T00:00:00Z host app 1 - - message {i:06}"))
                .collect()
        }

        type Harness = test_util::Harness<SyslogListener>;

        impl test_util::Transport for SyslogListener {
            async fn running(&self) -> bool {
                matches!(
                    &*self.transport.state.lock().await,
                    State::Running(running) if !running.intake.is_cancelled()
                )
            }
        }

        async fn start(udp: bool, drain_timeout: Duration) -> Harness {
            // Room for every batch, so the readers take all the datagrams off
            // the socket before the close.
            start_with(udp, drain_timeout, MESSAGES).await
        }

        async fn start_with(udp: bool, drain_timeout: Duration, buffer_cap: usize) -> Harness {
            let port = if udp { Port::Udp } else { Port::Tcp };
            Harness::start(port, drain_timeout, |addr, sender| {
                let transport = config::listener::SyslogListener {
                    udp: udp.then(|| config::listener::UdpListener {
                        address: addr,
                        max_packet_size: 2048,
                        buffer_cap,
                        recv_buffer_size: 1 << 20,
                        spin_before_block_us: 0,
                        worker_count: 1,
                        ip_whitelist: Vec::new(),
                        ip_blacklist: Vec::new(),
                    }),
                    tcp: (!udp).then(|| config::listener::TcpListener {
                        address: addr,
                        ip_whitelist: Vec::new(),
                        ip_blacklist: Vec::new(),
                    }),
                    tls: None,
                    idle_timeout: Duration::from_secs(60),
                };
                syslog_listener(transport, sender)
            })
            .await
        }

        /// Both UDP and TCP on `addr`'s port.
        async fn start_udp_and_tcp() -> Harness {
            Harness::start(Port::Tcp, DRAIN_TIMEOUT, |addr, sender| {
                let transport = config::listener::SyslogListener {
                    udp: Some(config::listener::UdpListener {
                        address: addr,
                        max_packet_size: 2048,
                        buffer_cap: MESSAGES,
                        recv_buffer_size: 1 << 20,
                        spin_before_block_us: 0,
                        worker_count: 1,
                        ip_whitelist: Vec::new(),
                        ip_blacklist: Vec::new(),
                    }),
                    tcp: Some(config::listener::TcpListener {
                        address: addr,
                        ip_whitelist: Vec::new(),
                        ip_blacklist: Vec::new(),
                    }),
                    tls: None,
                    idle_timeout: Duration::from_secs(60),
                };
                syslog_listener(transport, sender)
            })
            .await
        }

        fn syslog_listener(
            transport: config::listener::SyslogListener,
            sender: test_util::ProcessorSender,
        ) -> (config::listener::Listener, SyslogListener) {
            let listener_id = Uuid::now_v7();
            let conf = config::listener::Listener::new(
                listener_id,
                "syslog-close".into(),
                test_util::WORKERS,
                config::Batch {
                    batch_size: 10,
                    batch_timeout: Duration::from_millis(50),
                },
                config::listener::Config::Syslog(Box::new(config::listener::Syslog {
                    listener: transport,
                    cise: Default::default(),
                    cef: Default::default(),
                    cisco: Default::default(),
                })),
                false,
            );
            let listener =
                SyslogListener::new(conf.clone(), sender, None, item_for(listener_id), None)
                    .unwrap();
            (conf, listener)
        }

        /// The UDP readers of a running transport.
        async fn kill_udp_readers(h: &Harness) {
            if let State::Running(running) = &*h.listener.transport.state.lock().await {
                running.udp.as_ref().unwrap().kill_readers();
            }
        }

        fn connect(addr: SocketAddr) -> std::net::TcpStream {
            let deadline = std::time::Instant::now() + CLOSE_LIMIT;
            loop {
                match std::net::TcpStream::connect(addr) {
                    Ok(stream) => return stream,
                    Err(e) => {
                        assert!(std::time::Instant::now() < deadline, "connect failed: {e}");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn udp_close_under_load_delivers_everything_read() {
            let h = start(true, DRAIN_TIMEOUT).await;
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let msgs = messages();
            for m in &msgs {
                socket.send_to(m.as_bytes(), h.addr).unwrap();
            }
            let sent: u64 = msgs.iter().map(|m| m.len() as u64).sum();
            h.wait_for_intake(sent).await;
            let (_, rows) = h.close(true).await;
            assert_eq!(rows, Some(MESSAGES));
        }

        /// N11: with a channel too small for what was read, the readers are
        /// blocked on a full channel when the close comes, and used to drop
        /// the batches they held after counting them as taken in.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn udp_close_with_small_buffer_cap_delivers_everything() {
            let h = start_with(true, DRAIN_TIMEOUT, 1).await;
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let msgs = messages();
            let len = msgs[0].len();
            for m in &msgs {
                socket.send_to(m.as_bytes(), h.addr).unwrap();
            }
            h.wait_for_stalled_intake().await;
            let listener = h.listener.clone();
            let (_, rows) = h.close(true).await;
            let bytes = listener.transport.metrics();
            assert_eq!(bytes.total_bytes_dropped, 0);
            assert_eq!(rows, Some(bytes.total_bytes_succeeded as usize / len));
            assert!(
                rows.unwrap() > test_util::WORKERS * 10,
                "only {rows:?} rows: the readers never held anything"
            );
        }

        /// N10: the readers dying took no intake down with them, so a UDP-only
        /// transport handed its restarted workers the same dead channel.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn udp_only_transport_restarts_after_readers_die() {
            let h = start(true, DRAIN_TIMEOUT).await;
            let (rows, _consumer) = h.consume();
            kill_udp_readers(&h).await;
            h.wait_for_no_workers().await;
            h.restart_workers().await;
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            socket.send_to(messages()[0].as_bytes(), h.addr).unwrap();
            Harness::wait_for_rows(&rows, 1).await;
            h.close(false).await;
        }

        /// With TCP running too, its server kept the channel connected, so dead
        /// UDP readers went unnoticed and UDP intake stayed down for good.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn udp_dies_while_tcp_lives_restarts_udp() {
            let h = start_udp_and_tcp().await;
            let (rows, _consumer) = h.consume();
            kill_udp_readers(&h).await;
            h.wait_for_no_workers().await;
            h.restart_workers().await;
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            socket.send_to(messages()[0].as_bytes(), h.addr).unwrap();
            Harness::wait_for_rows(&rows, 1).await;
            h.close(false).await;
        }

        /// UDP bytes lived on the readers, so they read 0 once the transport
        /// closed and started over after a restart.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn udp_byte_counters_survive_close_and_restart() {
            let h = start(true, DRAIN_TIMEOUT).await;
            let (rows, _consumer) = h.consume();
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let msg = messages()[0].clone();
            socket.send_to(msg.as_bytes(), h.addr).unwrap();
            Harness::wait_for_rows(&rows, 1).await;
            let one = msg.len() as u64;
            assert_eq!(h.intake_bytes().await, one);
            kill_udp_readers(&h).await;
            h.wait_for_no_workers().await;
            h.restart_workers().await;
            socket.send_to(msg.as_bytes(), h.addr).unwrap();
            Harness::wait_for_rows(&rows, 2).await;
            h.wait_for_intake(2 * one).await;
            let listener = h.listener.clone();
            h.close(false).await;
            let bytes = listener.transport.metrics();
            assert_eq!(bytes.total_bytes, 2 * one);
            assert_eq!(bytes.total_bytes_succeeded, 2 * one);
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn tcp_close_under_load_delivers_everything_decoded() {
            let h = start(false, DRAIN_TIMEOUT).await;
            let addr = h.addr;
            let msgs = messages();
            let frame = msgs[0].len() as u64 + 1;
            // The client blocks once backpressure reaches it; the stream is
            // closed by the listener, not by the client.
            let client = std::thread::spawn(move || {
                let mut stream = connect(addr);
                for m in &msgs {
                    if stream.write_all(format!("{m}\n").as_bytes()).is_err() {
                        break;
                    }
                }
            });
            h.wait_for_stalled_intake().await;
            let listener = h.listener.clone();
            let (_, rows) = h.close(true).await;
            // The streams keep reading until the close reaches them, so count
            // what was decoded in total, after it.
            let decoded = listener.transport.metrics().total_bytes;
            assert_eq!(decoded % frame, 0, "intake stopped inside a frame");
            assert_eq!(rows, Some((decoded / frame) as usize));
            let _ = client.join();
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn close_with_a_stuck_pipeline_gives_up_after_the_drain_timeout() {
            let drain_timeout = Duration::from_millis(300);
            let h = start(true, drain_timeout).await;
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            for m in &messages() {
                socket.send_to(m.as_bytes(), h.addr).unwrap();
            }
            h.wait_for_stalled_intake().await;
            // The pipeline is never read: the drain cannot finish.
            let (elapsed, _) = h.close(false).await;
            assert!(
                elapsed >= drain_timeout,
                "returned before the drain timeout: {elapsed:?}"
            );
        }

        /// Workers come and go (a crash and restart, a scale to zero) while
        /// the transport keeps its socket and its connections.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn the_transport_outlives_its_workers() {
            let h = start(false, DRAIN_TIMEOUT).await;
            let (rows, _consumer) = h.consume();
            let line = format!("{}\n", messages()[0]);
            let mut client = connect(h.addr);
            client.write_all(line.as_bytes()).unwrap();
            Harness::wait_for_rows(&rows, 1).await;
            h.worker.adjust(0).await;
            assert_eq!(h.workers().await, 0);
            assert!(h.running().await);
            h.worker.adjust(2).await;
            client.write_all(line.as_bytes()).unwrap();
            Harness::wait_for_rows(&rows, 2).await;
            h.close(false).await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_closed_transport_stays_closed() {
            let h = start(false, DRAIN_TIMEOUT).await;
            let (addr, listener) = (h.addr, h.listener.clone());
            h.close(false).await;
            // A worker spawned just before the close must not start it again.
            assert!(listener.transport.ensure_started().await.unwrap().is_none());
            std::net::TcpListener::bind(addr).expect("port released");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn a_dead_intake_restarts_the_transport() {
            let h = start(false, DRAIN_TIMEOUT).await;
            let (rows, _consumer) = h.consume();
            // What a server that returns or panics does through its guard.
            if let State::Running(running) = &*h.listener.transport.state.lock().await {
                running.intake.cancel();
            }
            // The producers stop, so the workers' channel disconnects and
            // they return on their own.
            h.wait_for_no_workers().await;
            h.restart_workers().await;
            let mut client = connect(h.addr);
            client
                .write_all(format!("{}\n", messages()[0]).as_bytes())
                .unwrap();
            Harness::wait_for_rows(&rows, 1).await;
            h.close(false).await;
        }
    }

    #[tokio::test]
    async fn a_stream_sends_full_batches_and_partial_ones_after_its_linger() {
        use tokio::io::AsyncWriteExt;
        let transport = SyslogTransport::new(
            Uuid::nil(),
            "stream".into(),
            config::listener::SyslogListener {
                udp: None,
                tcp: None,
                tls: None,
                idle_timeout: Duration::ZERO,
            },
            config::Batch {
                batch_size: 4,
                batch_timeout: Duration::from_millis(200),
            },
        );
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let (tx, rx) = flume::bounded(16);
        let addr: SocketAddr = "127.0.0.1:514".parse().unwrap();
        let stream = tokio::spawn({
            let transport = transport.clone();
            async move {
                transport
                    .handle_syslog_stream(CancellationToken::new(), tx, server, addr)
                    .await
            }
        });
        // A quiet stream holds a partial batch for half of `batch_timeout`.
        let started = Instant::now();
        client.write_all(b"<13>one\n").await.unwrap();
        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv_async())
            .await
            .expect("partial batch not sent")
            .unwrap();
        assert_eq!(first.len(), 1);
        assert!(started.elapsed() >= Duration::from_millis(90));
        // A burst goes out in full batches; the rest when the peer is done.
        let frames: Vec<u8> = (0..10)
            .flat_map(|i| format!("<13>m{i}\n").into_bytes())
            .collect();
        client.write_all(&frames).await.unwrap();
        drop(client);
        stream.await.unwrap();
        let sizes: Vec<usize> = rx.drain().map(|batch| batch.len()).collect();
        assert_eq!(sizes, vec![4, 4, 2]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_closes_once_quiet_for_idle_timeout_and_sends_what_it_holds() {
        use tokio::io::AsyncWriteExt;
        let transport = SyslogTransport::new(
            Uuid::nil(),
            "stream".into(),
            config::listener::SyslogListener {
                udp: None,
                tcp: None,
                tls: None,
                idle_timeout: Duration::from_millis(100),
            },
            config::Batch {
                batch_size: 4,
                batch_timeout: Duration::from_secs(10),
            },
        );
        let (mut client, server) = tokio::io::duplex(1024);
        let (tx, rx) = flume::bounded(16);
        let addr: SocketAddr = "127.0.0.1:514".parse().unwrap();
        let stream = tokio::spawn(async move {
            transport
                .handle_syslog_stream(CancellationToken::new(), tx, server, addr)
                .await
        });
        sleep(Duration::from_millis(60)).await;
        client.write_all(b"<13>late\n").await.unwrap();
        // 90ms after the frame, 150ms after the start: not idle yet.
        sleep(Duration::from_millis(90)).await;
        assert!(!stream.is_finished());
        sleep(Duration::from_millis(20)).await;
        tokio::time::timeout(Duration::from_millis(1), stream)
            .await
            .expect("stream still open past idle_timeout")
            .unwrap();
        assert_eq!(rx.try_recv().unwrap().len(), 1);
    }

    fn fresh() -> SyslogCodec {
        SyslogCodec::new(Arc::default())
    }

    /// What `c` decoded since its last flush.
    fn snap(c: &SyslogCodec) -> (u64, u64, u64) {
        (c.tally.total, c.tally.succeeded, c.tally.dropped)
    }

    fn snap_shared(m: &ByteCounters) -> (u64, u64, u64) {
        (
            m.total_bytes.load(Ordering::Relaxed),
            m.total_bytes_succeeded.load(Ordering::Relaxed),
            m.total_bytes_dropped.load(Ordering::Relaxed),
        )
    }

    #[test]
    fn flush_moves_the_tally_into_the_shared_counters() {
        let m = Arc::new(ByteCounters::default());
        let mut c = SyslogCodec::new(m.clone());
        let mut buf = BytesMut::from(&b"5 hello\x01"[..]);
        c.decode(&mut buf).unwrap();
        assert_eq!(snap_shared(&m), (0, 0, 0));
        c.flush();
        assert_eq!(snap_shared(&m), (7, 7, 0));
        assert_eq!(snap(&c), (0, 0, 0));
        // the unknown framing byte is dropped, and dropping the codec flushes it
        assert!(c.decode(&mut buf).is_err());
        drop(c);
        assert_eq!(snap_shared(&m), (8, 7, 1));
    }

    // ----- octet-counted -----

    #[test]
    fn octet_counted_complete_frame() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"5 hello"[..]);
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"hello"[..]));
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (7, 7, 0));
    }

    #[test]
    fn octet_counted_two_frames_back_to_back() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"5 hello5 world"[..]);
        let r1 = c.decode(&mut buf).unwrap();
        let r2 = c.decode(&mut buf).unwrap();
        assert_eq!(r1.as_deref(), Some(&b"hello"[..]));
        assert_eq!(r2.as_deref(), Some(&b"world"[..]));
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (14, 14, 0));
    }

    #[test]
    fn octet_counted_partial_payload() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"5 hel"[..]);
        assert!(c.decode(&mut buf).unwrap().is_none());
        buf.extend_from_slice(b"lo");
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"hello"[..]));
        assert_eq!(snap(&c), (7, 7, 0));
    }

    #[test]
    fn octet_counted_partial_header() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"3"[..]);
        assert!(c.decode(&mut buf).unwrap().is_none());
        buf.extend_from_slice(b" hi!");
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"hi!"[..]));
        assert_eq!(snap(&c), (5, 5, 0));
    }

    #[test]
    fn octet_counted_invalid_prefix() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"12a 4abcd"[..]);
        let err = c.decode(&mut buf).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(buf.is_empty());
        let (total, succeeded, dropped) = snap(&c);
        assert_eq!(succeeded, 0);
        assert_eq!(dropped, 9);
        assert_eq!(total, 9);
    }

    #[test]
    fn octet_counted_prefix_too_long() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"12345678"[..]);
        let err = c.decode(&mut buf).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (8, 0, 8));
    }

    #[test]
    fn octet_counted_zero_length_rejected() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"0 "[..]);
        let err = c.decode(&mut buf).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (2, 0, 2));
    }

    #[test]
    fn octet_counted_oversize_rejected() {
        let mut c = fresh();
        let big = format!("{} payload", MAX_SYSLOG_MSG_SIZE + 1);
        let mut buf = BytesMut::from(big.as_bytes());
        let err = c.decode(&mut buf).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(buf.is_empty());
        let (_, succeeded, dropped) = snap(&c);
        assert_eq!(succeeded, 0);
        assert_eq!(dropped, big.len() as u64);
    }

    // ----- non-transparent -----

    #[test]
    fn non_transparent_lf() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"<13>hello\n"[..]);
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"<13>hello"[..]));
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (10, 10, 0));
    }

    #[test]
    fn non_transparent_lf_with_leftover() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"<13>a\nleftover"[..]);
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"<13>a"[..]));
        assert_eq!(&buf[..], b"leftover");
        assert_eq!(snap(&c), (6, 6, 0));
    }

    #[test]
    fn non_transparent_crlf_pair() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"<13>a\r\n"[..]);
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"<13>a"[..]));
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (7, 7, 0));
    }

    #[test]
    fn non_transparent_lone_cr() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"<13>a\rrest"[..]);
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"<13>a"[..]));
        assert_eq!(&buf[..], b"rest");
        assert_eq!(snap(&c), (6, 6, 0));
    }

    #[test]
    fn non_transparent_cr_at_tail_waits() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"<13>a\r"[..]);
        assert!(c.decode(&mut buf).unwrap().is_none());
        assert_eq!(snap(&c), (0, 0, 0));
        buf.extend_from_slice(b"\n");
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"<13>a"[..]));
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (7, 7, 0));
    }

    #[test]
    fn non_transparent_null_delimiter() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"<13>a\0"[..]);
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"<13>a"[..]));
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (6, 6, 0));
    }

    #[test]
    fn non_transparent_partial_under_cap() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"<13>partial"[..]);
        assert!(c.decode(&mut buf).unwrap().is_none());
        assert_eq!(snap(&c), (0, 0, 0));
    }

    #[test]
    fn non_transparent_oversize() {
        let mut c = fresh();
        let mut payload = Vec::with_capacity(MAX_SYSLOG_MSG_SIZE + 2);
        payload.push(b'<');
        payload.resize(MAX_SYSLOG_MSG_SIZE + 2, b'A');
        let len = payload.len();
        let mut buf = BytesMut::from(&payload[..]);
        let err = c.decode(&mut buf).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (len as u64, 0, len as u64));
    }

    // ----- unknown framing -----

    #[test]
    fn unknown_framing_clears_buffer() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"hello"[..]);
        let err = c.decode(&mut buf).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (5, 0, 5));
    }

    #[test]
    fn empty_buffer_returns_none() {
        let mut c = fresh();
        let mut buf = BytesMut::new();
        assert!(c.decode(&mut buf).unwrap().is_none());
        assert_eq!(snap(&c), (0, 0, 0));
    }

    // ----- raw CEF -----

    #[test]
    fn raw_cef_lf() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"CEF:0|V|P|1|2|n|3|dpt=443\n"[..]);
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"CEF:0|V|P|1|2|n|3|dpt=443"[..]));
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (26, 26, 0));
    }

    #[test]
    fn raw_cef_crlf() {
        let mut c = fresh();
        let mut buf = BytesMut::from(&b"CEF:0|V|P|1|2|n|3|\r\n"[..]);
        let r = c.decode(&mut buf).unwrap();
        assert_eq!(r.as_deref(), Some(&b"CEF:0|V|P|1|2|n|3|"[..]));
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (20, 20, 0));
    }

    #[test]
    fn raw_cef_mixed_with_syslog_framing() {
        let mut c = fresh();
        let mut buf =
            BytesMut::from(&b"CEF:0|V|P|1|2|n|3|\n<13>a\n5 helloCEF:1|V|P|1|2|n|3|\n"[..]);
        let frames: Vec<_> = std::iter::from_fn(|| c.decode(&mut buf).unwrap()).collect();
        assert_eq!(
            frames.iter().map(|b| &b[..]).collect::<Vec<_>>(),
            vec![
                &b"CEF:0|V|P|1|2|n|3|"[..],
                &b"<13>a"[..],
                &b"hello"[..],
                &b"CEF:1|V|P|1|2|n|3|"[..],
            ]
        );
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (51, 51, 0));
    }

    // ----- apply_cef -----

    fn cef_of(packet: &str) -> (syslog_loose::Message<&str>, Option<usize>) {
        let mut msg = syslog_loose::parse_message(packet, syslog_loose::Variant::Either);
        let off = apply_cef(packet, &mut msg).map(|(off, _)| off);
        (msg, off)
    }

    #[test]
    fn apply_cef_rfc3164_with_host() {
        let packet =
            "<134>Sep 19 08:26:10 fw01 CEF:0|Security|tm|1.0|100|worm stopped|10|src=10.0.0.1";
        let (msg, off) = cef_of(packet);
        assert_eq!(off, Some(26));
        assert_eq!(msg.hostname, Some("fw01"));
        assert_eq!(msg.appname, None);
    }

    #[test]
    fn apply_cef_rfc3164_without_host() {
        let packet = "<134>Sep 19 08:26:10 CEF:0|Security|tm|1.0|100|worm stopped|10|src=10.0.0.1";
        let (msg, off) = cef_of(packet);
        assert_eq!(off, Some(21));
        assert_eq!(msg.hostname, None);
        assert_eq!(msg.appname, None);
    }

    #[test]
    fn apply_cef_rfc5424() {
        let packet = "<134>1 2026-09-19T08:26:10Z fw01 app 42 ID7 - CEF:0|V|P|1|2|n|3|spt=1";
        let (msg, off) = cef_of(packet);
        assert_eq!(&packet[off.unwrap()..][..5], "CEF:0");
        assert_eq!(msg.hostname, Some("fw01"));
        assert_eq!(msg.appname, Some("app"));
        assert_eq!(msg.msgid, Some("ID7"));
        assert!(msg.procid.is_some());
    }

    #[test]
    fn apply_cef_raw() {
        let (msg, off) = cef_of("CEF:0|V|P|1|2|n|3|dpt=443");
        assert_eq!(off, Some(0));
        assert_eq!(msg.hostname, None);
        assert_eq!(msg.appname, None);
    }

    #[test]
    fn apply_cef_ignores_free_text_mention() {
        let packet = "<13>Sep 19 08:26:10 host app: saw CEF:0 in a note";
        let (msg, off) = cef_of(packet);
        assert_eq!(off, None);
        assert_eq!(msg.hostname, Some("host"));
        assert_eq!(msg.appname, Some("app"));
    }

    // ----- Cisco ISE -----

    // Made-up event: documentation addresses (RFC 5737), invented names.
    const ISE_SEG0: &str = "<181>Jan 12 03:04:05 ise-lab-01 CISE_TACACS_Accounting 0000004711 2 0 2026-01-12 03:04:05.678 +07:00 0000000042 3300 NOTICE Tacacs-Accounting: TACACS+ Accounting with Command, Device IP Address=192.0.2.10, User=alice, Step=13006, Step=15049,\n";
    const ISE_SEG1: &str = "<181>Jan 12 03:04:05 ise-lab-01 CISE_TACACS_Accounting 0000004711 2 1  TotalAuthenLatency=3, Response={AcctReply-Status=Success; },\n";

    /// Category, msg_id, total_seg and seg_num.
    type Counters<'a> = (&'a str, u64, u32, u32);

    fn cise_of(packet: &str) -> (syslog_loose::Message<&str>, Option<Counters<'_>>) {
        let (msg, format) = parse_message(packet, Detect::ALL);
        let seg = match format {
            Format::Cise(category, s) => Some((category, s.msg_id, s.total, s.num)),
            _ => None,
        };
        (msg, seg)
    }

    #[test]
    fn apply_cise_rfc3164_with_host() {
        let (msg, seg) = cise_of(ISE_SEG0);
        assert_eq!(seg, Some(("TACACS_Accounting", 4711, 2, 0)));
        assert_eq!(msg.hostname, Some("ise-lab-01"));
        assert_eq!(msg.appname, Some("CISE_TACACS_Accounting"));
        assert!(msg.msg.starts_with("0000004711 2 0 2026-01-12"));
    }

    #[test]
    fn apply_cise_rfc3164_without_host() {
        let packet = "<181>Jan 12 03:04:05 CSCOacs_Passed_Authentications 0000000042 1 0 2026-01-12 03:04:05.678 +07:00 1 5200 NOTICE Passed-Authentication: ok, a=b,";
        let (msg, seg) = cise_of(packet);
        assert_eq!(seg, Some(("Passed_Authentications", 42, 1, 0)));
        assert_eq!(msg.hostname, None);
        assert_eq!(msg.appname, Some("CSCOacs_Passed_Authentications"));
        assert_eq!(msg.procid, None);
        assert!(msg.msg.starts_with("0000000042 1 0 "));
    }

    #[test]
    fn apply_cise_needs_counters() {
        let (msg, seg) = cise_of("<181>Jan 12 03:04:05 ise-lab-01 CISE_Alarm: something happened");
        assert!(seg.is_none());
        assert_eq!(msg.appname, Some("CISE_Alarm"));
        let (_, seg) = cise_of("<13>Jan 12 03:04:05 host app: 1 2 0 not ise");
        assert!(seg.is_none());
    }

    #[test]
    fn apply_cise_off() {
        let (msg, format) = parse_message(
            ISE_SEG0,
            Detect {
                cise: false,
                ..Detect::ALL
            },
        );
        assert!(matches!(format, Format::Plain));
        assert_eq!(msg.appname, Some("CISE_TACACS_Accounting"));
    }

    fn ise_listener(
        cise: config::listener::Cise,
    ) -> (
        SyslogListener,
        crate::chan::AdminChannel<&'static str, payload::ProcessorPayload>,
    ) {
        listener_with(|c| c.cise = cise)
    }

    /// A listener on the default syslog config, as `edit` changes it.
    fn listener_with(
        edit: impl FnOnce(&mut config::listener::Syslog),
    ) -> (
        SyslogListener,
        crate::chan::AdminChannel<&'static str, payload::ProcessorPayload>,
    ) {
        let mut syslog = config::listener::Syslog {
            listener: config::listener::SyslogListener {
                udp: None,
                tcp: None,
                tls: None,
                idle_timeout: Duration::from_secs(60),
            },
            cise: Default::default(),
            cef: Default::default(),
            cisco: Default::default(),
        };
        edit(&mut syslog);
        let (admin, sender) = crate::chan::new(
            Duration::from_secs(3600),
            Arc::new(arc_swap::ArcSwap::from_pointee((
                std::time::Instant::now(),
                common::internal_metrics::Channel::default(),
            ))),
        );
        let conf = config::listener::Listener::new(
            Uuid::now_v7(),
            "syslog-ise".into(),
            1,
            config::Batch {
                batch_size: 100,
                batch_timeout: Duration::from_secs(1),
            },
            config::listener::Config::Syslog(Box::new(syslog)),
            false,
        );
        let listener = SyslogListener::new(conf, sender, None, cache::Cache::new(), None).unwrap();
        (listener, admin)
    }

    /// The batch's records as nested JSON objects.
    fn records(batch: &mut payload::LogsBuilder) -> Vec<serde_json::Value> {
        if batch.is_empty() {
            return Vec::new();
        }
        batch
            .finish()
            .serialize_as(config::Format::JsonNestedDenormalized)
            .unwrap()
            .iter()
            .flat_map(|b| {
                std::str::from_utf8(b)
                    .unwrap()
                    .lines()
                    .map(|l| serde_json::from_str(l).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn packets(texts: &[&str]) -> Packets {
        let addr: SocketAddr = "198.51.100.7:514".parse().unwrap();
        texts
            .iter()
            .map(|t| (Bytes::copy_from_slice(t.as_bytes()), addr))
            .collect()
    }

    #[tokio::test]
    async fn ise_segments_across_batches_make_one_record() {
        let (listener, _admin) = ise_listener(Default::default());
        let plain = "<13>Jan 12 03:04:05 host app: hello";
        let mut batch = payload::LogsBuilder::new();
        listener.parse_packets(&mut batch, packets(&[ISE_SEG0, plain]));
        let first = records(&mut batch);
        assert_eq!(first.len(), 1, "only the plain message: {first:#?}");
        assert!(first[0]["record"]["attributes"].get("format").is_none());

        listener.parse_packets(&mut batch, packets(&[ISE_SEG1]));
        let second = records(&mut batch);
        assert_eq!(second.len(), 1, "{second:#?}");
        let record = &second[0]["record"];
        // 2026-01-12 03:04:05.678 +07:00
        assert_eq!(record["observedTimeUnixNano"], "1768161845678000000");
        assert_eq!(record["severityText"], "SEVERITY_NUMBER_INFO2");
        assert_eq!(record["attributes"]["format"], "cise");
        assert_eq!(record["attributes"]["protocol"], "RFC3164");
        let json = record.to_string();
        for needle in [
            r#""category":"TACACS_Accounting""#,
            r#""msg_id":4711"#,
            r#""total_seg":2"#,
            r#""segments":2"#,
            r#""code":3300"#,
            r#""class":"Tacacs-Accounting""#,
            r#"{"key":"Device IP Address","value":"192.0.2.10"}"#,
            r#"{"key":"TotalAuthenLatency","value":"3"}"#,
            r#"{"key":"AcctReply-Status","value":"Success"}"#,
            r#""hostname":"ise-lab-01""#,
        ] {
            assert!(json.contains(needle), "missing {needle} in {json}");
        }
        assert!(listener.reassembler.lock().is_empty());
        let m = &listener.internal_metrics;
        assert_eq!(m.total_items_succeeded.load(Ordering::Relaxed), 3);
        assert_eq!(m.total_records.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn ise_incomplete_event_flushes_on_close() {
        let (listener, _admin) = ise_listener(Default::default());
        let mut batch = payload::LogsBuilder::new();
        listener.parse_packets(&mut batch, packets(&[ISE_SEG1]));
        assert!(records(&mut batch).is_empty());
        // Not timed out yet.
        listener.flush_reassembler(&mut batch, false);
        assert!(records(&mut batch).is_empty());
        listener.flush_reassembler(&mut batch, true);
        let out = records(&mut batch);
        assert_eq!(out.len(), 1);
        let json = out[0].to_string();
        assert!(json.contains(r#""segments":1"#), "{json}");
        assert!(
            json.contains(r#"{"key":"TotalAuthenLatency","value":"3"}"#),
            "{json}"
        );
    }

    #[tokio::test]
    async fn ise_reassembly_timeout_comes_from_config() {
        let (listener, _admin) = ise_listener(config::listener::Cise {
            reassembly_timeout: Duration::from_millis(1),
            ..Default::default()
        });
        let mut batch = payload::LogsBuilder::new();
        listener.parse_packets(&mut batch, packets(&[ISE_SEG0]));
        assert!(records(&mut batch).is_empty());
        std::thread::sleep(Duration::from_millis(5));
        listener.flush_reassembler(&mut batch, false);
        let out = records(&mut batch);
        assert_eq!(out.len(), 1);
        assert!(out[0].to_string().contains(r#""segments":1"#));
    }

    #[tokio::test]
    async fn ise_disabled_is_plain_syslog() {
        let (listener, _admin) = ise_listener(config::listener::Cise {
            enabled: false,
            ..Default::default()
        });
        let mut batch = payload::LogsBuilder::new();
        listener.parse_packets(&mut batch, packets(&[ISE_SEG0, ISE_SEG1]));
        let out = records(&mut batch);
        assert_eq!(out.len(), 2);
        for r in &out {
            assert!(r["record"]["attributes"].get("cise").is_none(), "{r}");
            assert!(r["record"]["attributes"].get("format").is_none(), "{r}");
        }
        assert!(listener.reassembler.lock().is_empty());
    }

    #[tokio::test]
    async fn cef_records_carry_format() {
        let (listener, _admin) = ise_listener(Default::default());
        let mut batch = payload::LogsBuilder::new();
        listener.parse_packets(
            &mut batch,
            packets(&[
                "<134>Jan 12 03:04:05 fw01 CEF:0|Security|tm|1.0|100|worm stopped|10|src=192.0.2.1",
                "CEF:0|Security|tm|1.0|100|worm stopped|10|src=192.0.2.1",
            ]),
        );
        let out = records(&mut batch);
        let seen: Vec<_> = out
            .iter()
            .map(|r| {
                let a = &r["record"]["attributes"];
                (
                    a["protocol"].as_str().unwrap(),
                    a["format"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(seen, [("RFC3164", "cef"), ("CEF", "cef")]);
    }

    #[tokio::test]
    async fn ise_segments_past_max_segments_stay_apart() {
        let (listener, _admin) = ise_listener(config::listener::Cise {
            max_segments: 1,
            ..Default::default()
        });
        let mut batch = payload::LogsBuilder::new();
        listener.parse_packets(&mut batch, packets(&[ISE_SEG0, ISE_SEG1]));
        let out = records(&mut batch);
        assert_eq!(out.len(), 2);
        for r in &out {
            let cise = &r["record"]["attributes"]["cise"];
            assert_eq!(cise["segments"], 1, "{r}");
            assert_eq!(cise["total_seg"], 2, "{r}");
        }
        assert!(listener.reassembler.lock().is_empty());
    }

    // ----- Cisco IOS / IOS XR / NX-OS / ASA / FTD -----

    /// One record per packet, attributes only.
    async fn cisco_records(texts: &[&str]) -> Vec<serde_json::Value> {
        records_with(|_| {}, texts).await
    }

    /// One record per packet, attributes only, from a listener `edit` configured.
    async fn records_with(
        edit: impl FnOnce(&mut config::listener::Syslog),
        texts: &[&str],
    ) -> Vec<serde_json::Value> {
        let (listener, _admin) = listener_with(edit);
        let mut batch = payload::LogsBuilder::new();
        listener.parse_packets(&mut batch, packets(texts));
        let out = records(&mut batch);
        assert_eq!(out.len(), texts.len(), "{out:#?}");
        out.into_iter().map(|r| r["record"].clone()).collect()
    }

    #[tokio::test]
    async fn cisco_ios_header_neither_rfc_parses() {
        let [r] = &cisco_records(
            &["<189>521: rtr-lab-01: *Jan 12 03:04:05.678: %LINK-5-CHANGED: Interface Gi2, changed state to down\n"],
        )
        .await[..] else {
            unreachable!()
        };
        let a = &r["attributes"];
        assert_eq!(a["format"], "cisco");
        assert_eq!(a["protocol"], "RFC3164");
        // Recovered from `<189>`.
        assert_eq!(a["facility_level"], "local7");
        assert_eq!(a["severity_level"], "notice");
        assert_eq!(a["hostname"], "rtr-lab-01");
        assert_eq!(a["cisco"]["facility"], "LINK");
        assert_eq!(a["cisco"]["severity"], 5);
        assert_eq!(a["cisco"]["mnemonic"], "CHANGED");
        assert_eq!(a["cisco"]["text"], "Interface Gi2, changed state to down");
        assert_eq!(a["cisco"]["seq"], 521);
        assert_eq!(a["cisco"]["clock_synced"], false);
        assert_eq!(
            r["body"],
            "%LINK-5-CHANGED: Interface Gi2, changed state to down"
        );
        assert_eq!(r["severityText"], "SEVERITY_NUMBER_INFO2");
    }

    #[tokio::test]
    async fn cisco_synced_device_time_is_the_observed_time() {
        let [r] = &cisco_records(
            &["<189>78: 000096: Jan 12 2026 03:04:05.678 UTC: %SYS-5-CONFIG_I: Configured from console by vty0"],
        )
        .await[..] else {
            unreachable!()
        };
        assert_eq!(r["observedTimeUnixNano"], "1768187045678000000");
        assert_eq!(r["attributes"]["cisco"]["seq"], 96);
        assert!(r["attributes"]["cisco"].get("clock_synced").is_none());
    }

    #[tokio::test]
    async fn cisco_iosxr() {
        let [r] = &cisco_records(
            &["<149>12: rtr-lab-02 RP/0/RSP0/CPU0:Jan 12 2026 03:04:05.678 UTC: bgp[1051]: %ROUTING-BGP-5-MAXPFX : No. of prefixes reached 90"],
        )
        .await[..] else {
            unreachable!()
        };
        let a = &r["attributes"];
        assert_eq!(a["hostname"], "rtr-lab-02");
        assert_eq!(a["cisco"]["facility"], "ROUTING");
        assert_eq!(a["cisco"]["subfacility"], "BGP");
        assert_eq!(a["cisco"]["node"], "RP/0/RSP0/CPU0");
        assert_eq!(a["cisco"]["process"], "bgp");
        assert_eq!(a["cisco"]["pid"], 1051);
    }

    #[tokio::test]
    async fn cisco_asa_and_ftd_behind_an_rfc3164_header() {
        let out = cisco_records(
            &[
                "<164>Jan 12 2026 03:04:05: %ASA-4-106023: Deny tcp src outside:192.0.2.1/4444 dst inside:198.51.100.2/22",
                "<166>Jan 12 2026 03:04:05 fw-lab-01 %FTD-6-430003: AccessControlRuleAction: Allow, SrcIP: 192.0.2.1, DstIP: 198.51.100.2",
            ],
        )
        .await;
        let asa = &out[0]["attributes"];
        // syslog_loose read the tag and the next word as host and app: both are cleared.
        assert!(asa.get("hostname").is_none(), "{asa}");
        assert!(asa.get("appname").is_none(), "{asa}");
        assert_eq!(asa["cisco"]["mnemonic"], "106023");
        assert!(asa["cisco"].get("attrs").is_none());
        assert_eq!(out[0]["severityText"], "SEVERITY_NUMBER_WARN");
        assert!(
            out[0]["body"]
                .as_str()
                .unwrap()
                .starts_with("%ASA-4-106023: Deny")
        );

        let ftd = &out[1]["attributes"];
        assert_eq!(ftd["hostname"], "fw-lab-01");
        assert!(ftd.get("appname").is_none(), "{ftd}");
        assert_eq!(
            ftd["cisco"]["attrs"],
            serde_json::json!([
                {"key": "AccessControlRuleAction", "value": "Allow"},
                {"key": "SrcIP", "value": "192.0.2.1"},
                {"key": "DstIP", "value": "198.51.100.2"},
            ])
        );
    }

    #[tokio::test]
    async fn cisco_disabled_is_plain_syslog() {
        let [r] = &records_with(
            |c| c.cisco.enabled = false,
            &["<189>521: rtr-lab-01: *Jan 12 03:04:05.678: %LINK-5-CHANGED: Interface Gi2"],
        )
        .await[..] else {
            unreachable!()
        };
        let a = &r["attributes"];
        assert!(a.get("cisco").is_none() && a.get("format").is_none(), "{a}");
        assert!(a.get("hostname").is_none(), "{a}");
        // The PRI is still recovered.
        assert_eq!(a["facility_level"], "local7");
        assert_eq!(
            r["body"],
            "521: rtr-lab-01: *Jan 12 03:04:05.678: %LINK-5-CHANGED: Interface Gi2"
        );
    }

    #[tokio::test]
    async fn cef_disabled_is_plain_syslog() {
        let packet =
            "<134>Jan 12 03:04:05 fw01 CEF:0|Security|tm|1.0|100|worm stopped|10|src=192.0.2.1";
        let out = records_with(
            |c| c.cef.enabled = false,
            &[
                packet,
                "CEF:0|Security|tm|1.0|100|worm stopped|10|src=192.0.2.1",
            ],
        )
        .await;
        for r in &out {
            let a = &r["attributes"];
            assert!(a.get("cef").is_none() && a.get("format").is_none(), "{a}");
            // Raw CEF has no header to read, so it is not `CEF` either.
            assert_eq!(a["protocol"], "RFC3164", "{a}");
        }
        // The syslog severity (info), not the CEF one (10).
        assert_eq!(out[0]["severityText"], "SEVERITY_NUMBER_INFO");
    }

    #[tokio::test]
    async fn pri_is_recovered_for_any_unparsed_header() {
        let out = cisco_records(&[
            "<13>free text with no header",
            "<999>not a pri",
            "no pri at all",
        ])
        .await;
        assert_eq!(out[0]["attributes"]["facility_level"], "user");
        assert_eq!(out[0]["attributes"]["severity_level"], "notice");
        assert_eq!(out[0]["body"], "free text with no header");
        assert!(out[0]["attributes"].get("format").is_none());
        for r in &out[1..] {
            assert!(r["attributes"].get("facility_level").is_none(), "{r}");
        }
        assert_eq!(out[1]["body"], "<999>not a pri");
        assert_eq!(out[2]["body"], "no pri at all");
    }

    #[test]
    fn cef_and_ise_win_over_cisco() {
        let (_, format) = parse_message(
            "<134>Jan 12 03:04:05 fw01 CEF:0|V|P|1|2|%SYS-5-X: n|3|src=192.0.2.1",
            Detect::ALL,
        );
        assert!(matches!(format, Format::Cef(..)));
        let (_, format) = parse_message(ISE_SEG0, Detect::ALL);
        assert!(matches!(format, Format::Cise(..)));
    }

    // ----- mixed sequence invariant -----

    #[test]
    fn mixed_sequence_totals() {
        let mut c = fresh();
        let mut buf = BytesMut::from("5 hello<13>사랑해\n");
        let r1 = c.decode(&mut buf).unwrap();
        let r2 = c.decode(&mut buf).unwrap();
        assert_eq!(r1.as_deref(), Some(&b"hello"[..]));
        assert_eq!(
            r2.map(|b| String::from_utf8_lossy(&b).to_string()),
            Some("<13>사랑해".to_string())
        );
        assert!(buf.is_empty());
        assert_eq!(snap(&c), (21, 21, 0));
    }
}
