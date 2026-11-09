// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! UDP receive path shared by the datagram listeners (syslog, netflow/sFlow).
//!
//! Each worker owns one `SO_REUSEPORT` socket and one dedicated OS thread that
//! blocks in `recvmmsg`. The thread never shares a core with tokio tasks, so a
//! runtime busy parsing cannot starve the receive queue: the kernel buffer
//! only has to absorb the thread's wake-up latency, not a scheduling stall.
//! That is what keeps `RcvbufErrors` near zero without raising
//! `net.core.rmem_max`.
//!
//! `recvmmsg` is Linux-only. Elsewhere (macOS) the same slots are filled by
//! one `recvmsg` per datagram, which keeps the behavior and costs a syscall
//! per datagram: good for development, not tuned for throughput.

use anyhow::Result;
use bytes::{Bytes, BytesMut};
use flume::{SendTimeoutError, Sender};
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use nix::sys::socket::recvmsg;
use nix::sys::socket::{MsgFlags, SockaddrStorage};
#[cfg(any(target_os = "linux", target_os = "android"))]
use nix::sys::socket::{MultiHeaders, recvmmsg};
use socket2::{Domain, Protocol, Socket, Type};
use std::io::{self, IoSliceMut};
use std::net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Datagrams pulled per `recvmmsg` call. Below saturation a call finds about
/// one datagram queued; once the reader falls behind, batching is what lets it
/// catch up. One socket fed ~3.8M pps lost 0.2–3% reading one datagram per
/// call, and ~0% batched.
const RECV_VLEN: usize = 64;
/// Longest a blocked receive or send waits before rechecking the cancel token
/// and the batch deadline.
const MAX_TICK: Duration = Duration::from_millis(100);
/// Datagrams counted locally before the shared atomics are updated.
const METRICS_FLUSH_EVERY: u32 = 64;
/// Datagrams between looks at the cancel token while traffic is flowing; an
/// idle socket also looks on every read timeout. `is_cancelled` takes a lock,
/// which on every datagram is a measurable share of the per-packet cost.
const CANCEL_CHECK_EVERY: u32 = 256;
/// Packets are copied out of the receive buffer into chunks of this size, so a
/// batch pins roughly its own bytes rather than `max_packet_size` per datagram.
const ARENA_SIZE: usize = 256 << 10;

type Batch = Vec<(Bytes, SocketAddr)>;

/// Per-thread receive state. nix's `MultiHeaders` holds raw pointers and is
/// not `Send`, so it lives on the reader thread rather than in `UdpListener`.
#[cfg(any(target_os = "linux", target_os = "android"))]
type RecvHeaders = MultiHeaders<SockaddrStorage>;
/// `recvmsg` needs no state that outlives a call.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
struct RecvHeaders;

#[cfg(any(target_os = "linux", target_os = "android"))]
fn recv_headers() -> RecvHeaders {
    MultiHeaders::preallocate(RECV_VLEN, None)
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn recv_headers() -> RecvHeaders {
    RecvHeaders
}

struct Datagram {
    len: usize,
    addr: Option<SocketAddr>,
    truncated: bool,
}

/// Counted per worker and published every few datagrams: the shared atomics
/// are written by every worker of the listener. `total` and the filtered
/// `dropped` are booked as a datagram is read, `succeeded` (or `dropped`)
/// once its batch is handed over or given up.
#[derive(Default)]
struct PendingMetrics {
    datagrams: u32,
    total: u64,
    succeeded: u64,
    dropped: u64,
    truncated: bool,
}

struct UdpListener {
    socket: Socket,
    max_packet_size: usize,
    /// `RECV_VLEN` slots of `max_packet_size` bytes, one per datagram.
    buf: Vec<u8>,
    /// What the last `recv` call returned, one entry per filled slot.
    dgrams: Vec<Datagram>,
    arena: BytesMut,
    /// How long to keep polling an empty socket before sleeping. Waking from a
    /// deep C-state can take hundreds of microseconds, which near 1M pps is
    /// all the headroom a stock-sized receive buffer has; polling bridges the
    /// gaps between back-to-back datagrams so the core never idles. Each
    /// polled gap is CPU taken from the rest of the process, so it is opt-in.
    spin_before_block: Duration,
    batch_size: usize,
    batch_timeout: Duration,
    ip_whitelist: Arc<Vec<IpAddr>>,
    ip_blacklist: Arc<Vec<IpAddr>>,

    /// The last receive left slots unused, so it already emptied the queue.
    drained: bool,
    pending: PendingMetrics,
    internal_metrics: Arc<ByteCounters>,
}

impl UdpListener {
    #[allow(clippy::too_many_arguments)]
    fn new(
        addr: SocketAddr,
        max_packet_size: usize,
        recv_buffer_size: usize,
        spin_before_block: Duration,
        batch_size: usize,
        batch_timeout: Duration,
        ip_whitelist: Arc<Vec<IpAddr>>,
        ip_blacklist: Arc<Vec<IpAddr>>,
        internal_metrics: Arc<ByteCounters>,
    ) -> Result<Self> {
        let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_port(true)?;
        socket.set_reuse_address(true)?;
        set_recv_buffer(&socket, recv_buffer_size);
        socket.bind(&addr.into())?;
        // Blocking socket: the timeout bounds how long a quiet socket sleeps
        // before the loop looks at the batch deadline and the cancel token.
        socket.set_read_timeout(Some(
            (batch_timeout / 2).clamp(Duration::from_millis(1), MAX_TICK),
        ))?;
        Ok(Self {
            socket,
            max_packet_size: max_packet_size.max(1),
            buf: vec![0; RECV_VLEN * max_packet_size.max(1)],
            dgrams: Vec::with_capacity(RECV_VLEN),
            arena: BytesMut::new(),
            spin_before_block,
            batch_size,
            batch_timeout,
            ip_whitelist,
            ip_blacklist,
            drained: true,
            pending: PendingMetrics::default(),
            internal_metrics,
        })
    }

    fn is_allowed(&self, ip: &IpAddr) -> bool {
        if !self.ip_whitelist.is_empty() && self.ip_whitelist.binary_search(ip).is_err() {
            return false;
        }
        if !self.ip_blacklist.is_empty() && self.ip_blacklist.binary_search(ip).is_ok() {
            return false;
        }
        true
    }

    /// Fills `buf` slot by slot and records each datagram in `dgrams`. With
    /// `block`, waits for the first one up to the socket's read timeout, then
    /// takes whatever else is already queued; without, fails with
    /// `WouldBlock` when nothing is queued.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn recv(&mut self, headers: &mut RecvHeaders, block: bool) -> io::Result<usize> {
        let flags = if block {
            MsgFlags::MSG_WAITFORONE
        } else {
            MsgFlags::MSG_DONTWAIT
        };
        let mut slots = self.buf.chunks_exact_mut(self.max_packet_size);
        let mut iovs: [[IoSliceMut<'_>; 1]; RECV_VLEN] =
            std::array::from_fn(|_| [IoSliceMut::new(slots.next().unwrap())]);
        // The kernel's own `recvmmsg` timeout is only checked between
        // datagrams, so the socket's read timeout bounds the wait instead.
        let received = recvmmsg(
            self.socket.as_raw_fd(),
            headers,
            iovs.iter_mut(),
            flags,
            None,
        )?;
        // `received` borrows `buf`, so copy out what each slot holds before
        // anything reads the bytes.
        self.dgrams.clear();
        self.dgrams.extend(received.map(|msg| Datagram {
            len: msg.bytes,
            addr: msg.address.as_ref().and_then(to_socket_addr),
            truncated: msg.flags.contains(MsgFlags::MSG_TRUNC),
        }));
        Ok(self.dgrams.len())
    }

    /// Same contract as the `recvmmsg` version, one `recvmsg` per slot: only
    /// the first may wait, the rest take what is already queued and stop at
    /// the first miss, as `MSG_WAITFORONE` does.
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn recv(&mut self, _headers: &mut RecvHeaders, block: bool) -> io::Result<usize> {
        self.dgrams.clear();
        for (i, slot) in self.buf.chunks_exact_mut(self.max_packet_size).enumerate() {
            let flags = if block && i == 0 {
                MsgFlags::empty()
            } else {
                MsgFlags::MSG_DONTWAIT
            };
            let mut iov = [IoSliceMut::new(slot)];
            match recvmsg::<SockaddrStorage>(self.socket.as_raw_fd(), &mut iov, None, flags) {
                Ok(msg) => self.dgrams.push(Datagram {
                    len: msg.bytes,
                    addr: msg.address.as_ref().and_then(to_socket_addr),
                    truncated: msg.flags.contains(MsgFlags::MSG_TRUNC),
                }),
                Err(e) if i == 0 => return Err(e.into()),
                // Usually EAGAIN: the queue is empty. Any other error recurs
                // on the next call, after the datagrams already read are
                // delivered.
                Err(_) => break,
            }
        }
        Ok(self.dgrams.len())
    }

    /// Takes what is queued, sleeping in a blocking receive when nothing is.
    ///
    /// A call that returned fewer than `RECV_VLEN` datagrams already emptied
    /// the queue (`MSG_WAITFORONE` stops at the first miss), so the next call
    /// goes straight to the blocking receive instead of first making a
    /// non-blocking one that would fail. With `spin_before_block` set, the
    /// non-blocking polls are the point and always run.
    fn recv_next(&mut self, headers: &mut RecvHeaders) -> io::Result<usize> {
        let res = if self.drained && self.spin_before_block.is_zero() {
            // About to sleep: publish what this worker has counted.
            self.flush_metrics();
            self.recv(headers, true)
        } else {
            self.poll_then_block(headers)
        };
        self.drained = !matches!(res, Ok(n) if n == RECV_VLEN);
        res
    }

    fn poll_then_block(&mut self, headers: &mut RecvHeaders) -> io::Result<usize> {
        let mut started = None;
        loop {
            match self.recv(headers, false) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if self.spin_before_block.is_zero()
                        || started.get_or_insert_with(Instant::now).elapsed()
                            >= self.spin_before_block
                    {
                        // About to sleep: publish what this worker has counted.
                        self.flush_metrics();
                        return self.recv(headers, true);
                    }
                    std::hint::spin_loop();
                }
                res => return res,
            }
        }
    }

    fn flush_metrics(&mut self) {
        let p = std::mem::take(&mut self.pending);
        // A send books its bytes apart from the datagrams read.
        if p.datagrams == 0 && p.succeeded == 0 {
            return;
        }
        self.internal_metrics.add(p.total, p.succeeded, p.dropped);
        if p.truncated {
            warn!(
                max_packet_size = self.max_packet_size,
                "datagram larger than max_packet_size truncated"
            );
        }
    }

    fn run(mut self, sender: Sender<Batch>, token: CancellationToken) {
        let mut headers = recv_headers();
        let mut batch = Vec::with_capacity(self.batch_size);
        let mut batch_started = Instant::now();
        let mut since_cancel_check = 0u32;
        // Batches a cancelled send gave back. Their datagrams are already off
        // the kernel queue, so they are handed over once the socket is gone.
        let mut held: Vec<Batch> = Vec::new();
        'recv: loop {
            match self.recv_next(&mut headers) {
                Ok(n) => {
                    for i in 0..n {
                        let Datagram {
                            len,
                            addr,
                            truncated,
                        } = self.dgrams[i];
                        let len = len.min(self.max_packet_size);
                        let p = &mut self.pending;
                        p.datagrams += 1;
                        p.total += len as u64;
                        p.truncated |= truncated;
                        let Some(addr) = addr.filter(|a| self.is_allowed(&a.ip())) else {
                            self.pending.dropped += len as u64;
                            continue;
                        };
                        if batch.is_empty() {
                            batch_started = Instant::now();
                        }
                        let start = i * self.max_packet_size;
                        let packet = copy_out(&mut self.arena, &self.buf[start..start + len]);
                        batch.push((packet, addr));
                        if batch.len() >= self.batch_size {
                            let full =
                                std::mem::replace(&mut batch, Vec::with_capacity(self.batch_size));
                            if !held.is_empty() {
                                held.push(full);
                            } else if let Err(full) = self.send(&sender, full, &token) {
                                // Keep going through the slots already read.
                                held.push(full);
                            }
                        }
                    }
                    if !held.is_empty() {
                        break 'recv;
                    }
                    if self.pending.datagrams >= METRICS_FLUSH_EVERY {
                        self.flush_metrics();
                    }
                    since_cancel_check += n as u32;
                    if since_cancel_check >= CANCEL_CHECK_EVERY {
                        since_cancel_check = 0;
                        if token.is_cancelled() {
                            break;
                        }
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    if token.is_cancelled() {
                        break;
                    }
                }
                Err(e) => {
                    error!(error = %e, "udp worker stopped on recv error");
                    break;
                }
            }
            if !batch.is_empty() && batch_started.elapsed() >= self.batch_timeout {
                let partial = std::mem::replace(&mut batch, Vec::with_capacity(self.batch_size));
                if let Err(partial) = self.send(&sender, partial, &token) {
                    held.push(partial);
                    break;
                }
            }
        }
        held.push(batch);
        self.flush_metrics();
        let metrics = self.internal_metrics.clone();
        // Free the port before waiting on the consumers.
        drop(self);
        // Blocks until the consumers take each batch. That is bounded: the
        // listener's close stops its intake while the workers still read, and
        // past its drain timeout drops every receiver, which disconnects this.
        let (mut succeeded, mut dropped) = (0, 0);
        for batch in held.into_iter().filter(|b| !b.is_empty()) {
            let bytes = batch_bytes(&batch);
            match sender.send(batch) {
                Ok(()) => succeeded += bytes,
                Err(_) => dropped += bytes,
            }
        }
        metrics.add(0, succeeded, dropped);
    }

    /// Hands a batch to the consumers, blocking while the channel is full so
    /// its capacity stays the only buffering past the kernel. Gives the batch
    /// back once the worker should stop.
    fn send(
        &mut self,
        sender: &Sender<Batch>,
        mut batch: Batch,
        token: &CancellationToken,
    ) -> Result<(), Batch> {
        let bytes = batch_bytes(&batch);
        loop {
            match sender.send_timeout(batch, MAX_TICK) {
                Ok(()) => {
                    self.pending.succeeded += bytes;
                    return Ok(());
                }
                Err(SendTimeoutError::Timeout(b)) => {
                    if token.is_cancelled() {
                        return Err(b);
                    }
                    // Stuck on a full channel, maybe before the first flush:
                    // publish what this worker has read meanwhile.
                    self.flush_metrics();
                    batch = b;
                }
                Err(SendTimeoutError::Disconnected(b)) => return Err(b),
            }
        }
    }
}

fn batch_bytes(batch: &Batch) -> u64 {
    batch.iter().map(|(p, _)| p.len() as u64).sum()
}

fn to_socket_addr(addr: &SockaddrStorage) -> Option<SocketAddr> {
    if let Some(v4) = addr.as_sockaddr_in() {
        return Some(SocketAddr::V4(SocketAddrV4::from(*v4)));
    }
    addr.as_sockaddr_in6()
        .map(|v6| SocketAddr::V6(SocketAddrV6::from(*v6)))
}

fn copy_out(arena: &mut BytesMut, data: &[u8]) -> Bytes {
    if arena.capacity() - arena.len() < data.len() {
        *arena = BytesMut::with_capacity(ARENA_SIZE.max(data.len()));
    }
    arena.extend_from_slice(data);
    arena.split().freeze()
}

/// `SO_RCVBUFFORCE` gets `size` in full when the process has `CAP_NET_ADMIN`;
/// otherwise `SO_RCVBUF` silently clamps it to `net.core.rmem_max`, which still
/// beats `rmem_default`.
fn set_recv_buffer(socket: &Socket, size: usize) {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if nix::sys::socket::setsockopt(socket, nix::sys::socket::sockopt::RcvBufForce, &size).is_ok() {
        return;
    }
    if let Err(e) = socket.set_recv_buffer_size(size) {
        warn!(error = %e, "failed to raise udp receive buffer");
    }
}

/// Byte counters of a listener's intake. The listener owns them, so they
/// outlive each transport it starts.
#[derive(Default)]
pub struct ByteCounters {
    pub total_bytes: AtomicU64,
    pub total_bytes_succeeded: AtomicU64,
    pub total_bytes_dropped: AtomicU64,
}

impl ByteCounters {
    pub fn add(&self, total: u64, succeeded: u64, dropped: u64) {
        for (counter, n) in [
            (&self.total_bytes, total),
            (&self.total_bytes_succeeded, succeeded),
            (&self.total_bytes_dropped, dropped),
        ] {
            if n > 0 {
                counter.fetch_add(n, Ordering::Relaxed);
            }
        }
    }

    pub fn get(&self) -> ByteCounts {
        ByteCounts {
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            total_bytes_succeeded: self.total_bytes_succeeded.load(Ordering::Relaxed),
            total_bytes_dropped: self.total_bytes_dropped.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteCounts {
    pub total_bytes: u64,
    pub total_bytes_succeeded: u64,
    pub total_bytes_dropped: u64,
}

pub struct UdpListenerWorker {
    listeners: Vec<JoinHandle<()>>,
    token: CancellationToken,

    internal_metrics: Arc<ByteCounters>,
}

impl UdpListenerWorker {
    /// Starts `worker_count` reader threads that send their batches into
    /// `sender`. The caller owns the channel: its capacity is the buffering
    /// past the kernel, and it disconnects once every reader has stopped and
    /// the caller's own senders are gone. The readers stop on `close`, on drop,
    /// or once `parent` is cancelled; a reader that stops on its own cancels
    /// `parent`, so the caller's intake restarts whole. Bytes are added to
    /// `internal_metrics`, which the caller keeps across restarts.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task: (Uuid, String),
        address: SocketAddr,
        sender: Sender<Batch>,
        parent: &CancellationToken,
        internal_metrics: Arc<ByteCounters>,
        worker_count: usize,
        max_packet_size: usize,
        recv_buffer_size: usize,
        spin_before_block: Duration,
        batch_size: usize,
        batch_timeout: Duration,
        ip_whitelist: Vec<IpAddr>,
        ip_blacklist: Vec<IpAddr>,
    ) -> Result<Self> {
        let token = parent.child_token();
        let mut listeners = Vec::with_capacity(worker_count);
        let mut udp_listeners = Vec::with_capacity(worker_count);
        let ip_whitelist = Arc::new(ip_whitelist);
        let ip_blacklist = Arc::new(ip_blacklist);
        for _ in 0..worker_count {
            let internal_metrics = internal_metrics.clone();
            let listener = UdpListener::new(
                address,
                max_packet_size,
                recv_buffer_size,
                spin_before_block,
                batch_size,
                batch_timeout,
                ip_whitelist.clone(),
                ip_blacklist.clone(),
                internal_metrics,
            )?;
            udp_listeners.push(listener);
        }
        for (worker_id, listener) in udp_listeners.into_iter().enumerate() {
            let sender = sender.clone();
            let worker_token = token.clone();
            let intake = parent.clone();
            let span = tracing::info_span!(
                "udp_listen",
                listener_id = %task.0,
                listener = %task.1,
                worker = worker_id
            );
            let spawned = std::thread::Builder::new()
                .name(format!("udp-recv-{worker_id}"))
                .spawn(move || {
                    let _span = span.entered();
                    // However this returns, the rest of the intake goes down with it.
                    let _intake_guard = intake.drop_guard();
                    info!(
                        addr = %listener.socket.local_addr().ok().and_then(|a| a.as_socket()).unwrap_or(address),
                        recv_buffer = listener.socket.recv_buffer_size().unwrap_or(0),
                        "udp listening"
                    );
                    listener.run(sender, worker_token);
                    debug!("udp worker stopped");
                });
            match spawned {
                Ok(handle) => listeners.push(handle),
                Err(e) => {
                    // Stop the workers already running before giving up.
                    token.cancel();
                    return Err(e.into());
                }
            }
        }
        Ok(Self {
            listeners,
            token,
            internal_metrics,
        })
    }

    pub fn get_metrics(&self) -> ByteCounts {
        self.internal_metrics.get()
    }

    /// Stops the readers as if they had died on their own.
    #[cfg(test)]
    pub(crate) fn kill_readers(&self) {
        self.token.cancel();
    }

    pub async fn close(mut self) {
        self.token.cancel();
        let listeners = std::mem::take(&mut self.listeners);
        // Workers notice the token within one read timeout; join off the runtime.
        let panicked = tokio::task::spawn_blocking(move || {
            listeners
                .into_iter()
                .map(|handle| handle.join().is_err())
                .fold(false, |acc, panicked| acc | panicked)
        })
        .await;
        if !matches!(panicked, Ok(false)) {
            error!("udp worker panicked");
        }
    }
}

impl Drop for UdpListenerWorker {
    /// A worker left running keeps its `SO_REUSEPORT` socket bound and silently
    /// takes a share of the port's traffic, so never let one outlive us.
    fn drop(&mut self) {
        self.token.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flume::Receiver;
    use std::net::UdpSocket as StdUdpSocket;

    fn worker(
        batch_size: usize,
        batch_timeout: Duration,
        max_packet_size: usize,
        ip_whitelist: Vec<IpAddr>,
    ) -> (UdpListenerWorker, SocketAddr, Receiver<Batch>) {
        let port = StdUdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let (tx, rx) = flume::bounded(16);
        let w = UdpListenerWorker::new(
            (Uuid::now_v7(), "test".into()),
            addr,
            tx,
            &CancellationToken::new(),
            Arc::default(),
            2,
            max_packet_size,
            1 << 20,
            Duration::from_micros(2),
            batch_size,
            batch_timeout,
            ip_whitelist,
            Vec::new(),
        )
        .unwrap();
        (w, addr, rx)
    }

    async fn collect(rx: &Receiver<Batch>, want: usize) -> Vec<Batch> {
        let mut batches = Vec::new();
        let mut got = 0;
        while got < want {
            let batch = tokio::time::timeout(Duration::from_secs(5), rx.recv_async())
                .await
                .expect("timed out waiting for datagrams")
                .unwrap();
            got += batch.len();
            batches.push(batch);
        }
        batches
    }

    #[tokio::test]
    async fn delivers_datagrams_with_source() {
        let (w, addr, rx) = worker(4, Duration::from_millis(50), 64, Vec::new());
        let client = StdUdpSocket::bind("127.0.0.1:0").unwrap();
        for i in 0..10u8 {
            client.send_to(&[i; 8], addr).unwrap();
        }
        let batches = collect(&rx, 10).await;
        assert!(
            batches.iter().all(|b| b.len() <= 4),
            "batch over batch_size"
        );
        let mut seen: Vec<u8> = batches
            .iter()
            .flatten()
            .map(|(p, src)| {
                assert_eq!(*src, client.local_addr().unwrap());
                assert_eq!(p.len(), 8);
                p[0]
            })
            .collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..10).collect::<Vec<_>>());
        // Bytes are booked once their batch is handed over, so the reader may
        // publish the last of them just after the batch arrives.
        let deadline = Instant::now() + Duration::from_secs(2);
        while w.get_metrics().total_bytes_succeeded < 80 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let m = w.get_metrics();
        assert_eq!(m.total_bytes, 80);
        assert_eq!(m.total_bytes_succeeded, 80);
        w.close().await;
    }

    #[tokio::test]
    async fn delivers_ipv6_source() {
        let port = StdUdpSocket::bind("[::1]:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let addr: SocketAddr = format!("[::1]:{port}").parse().unwrap();
        let (tx, rx) = flume::bounded(16);
        let w = UdpListenerWorker::new(
            (Uuid::now_v7(), "test".into()),
            addr,
            tx,
            &CancellationToken::new(),
            Arc::default(),
            1,
            64,
            1 << 20,
            Duration::ZERO,
            3,
            Duration::from_millis(50),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let client = StdUdpSocket::bind("[::1]:0").unwrap();
        for _ in 0..3 {
            client.send_to(b"v6", addr).unwrap();
        }
        let batches = collect(&rx, 3).await;
        for (p, src) in batches.iter().flatten() {
            assert_eq!(p.as_ref(), b"v6");
            assert_eq!(*src, client.local_addr().unwrap());
        }
        w.close().await;
    }

    #[tokio::test]
    async fn partial_batch_flushes_on_timeout() {
        let (w, addr, rx) = worker(1000, Duration::from_millis(50), 64, Vec::new());
        let client = StdUdpSocket::bind("127.0.0.1:0").unwrap();
        client.send_to(b"one", addr).unwrap();
        let started = Instant::now();
        let batches = collect(&rx, 1).await;
        assert_eq!(batches[0][0].0.as_ref(), b"one");
        assert!(started.elapsed() < Duration::from_secs(1));
        w.close().await;
    }

    #[tokio::test]
    async fn truncates_to_max_packet_size() {
        let (w, addr, rx) = worker(1, Duration::from_millis(50), 16, Vec::new());
        let client = StdUdpSocket::bind("127.0.0.1:0").unwrap();
        client.send_to(&[7u8; 100], addr).unwrap();
        let batches = collect(&rx, 1).await;
        assert_eq!(batches[0][0].0.as_ref(), &[7u8; 16]);
        w.close().await;
    }

    // 127.0.0.1 is the only loopback source every platform has (macOS does not
    // route the rest of 127/8), so the whitelist is varied instead.
    #[tokio::test]
    async fn whitelist_drops_other_sources() {
        let elsewhere: IpAddr = "192.0.2.1".parse().unwrap();
        let (w, addr, _rx) = worker(1, Duration::from_millis(50), 64, vec![elsewhere]);
        StdUdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .send_to(b"blocked", addr)
            .unwrap();
        // The datagram may have hashed to either worker's socket.
        let deadline = Instant::now() + Duration::from_secs(2);
        while w.get_metrics().total_bytes_dropped == 0 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let m = w.get_metrics();
        assert_eq!(m.total_bytes_dropped, 7);
        assert_eq!(m.total_bytes_succeeded, 0);
        w.close().await;

        let local: IpAddr = "127.0.0.1".parse().unwrap();
        let (w, addr, rx) = worker(1, Duration::from_millis(50), 64, vec![local]);
        StdUdpSocket::bind((local, 0))
            .unwrap()
            .send_to(b"allowed", addr)
            .unwrap();
        let batches = collect(&rx, 1).await;
        assert_eq!(batches[0][0].0.as_ref(), b"allowed");
        assert_eq!(w.get_metrics().total_bytes_dropped, 0);
        w.close().await;
    }

    #[tokio::test]
    async fn close_stops_workers_and_releases_port() {
        let (w, addr, _rx) = worker(100, Duration::from_secs(1), 64, Vec::new());
        let started = Instant::now();
        w.close().await;
        assert!(started.elapsed() < Duration::from_secs(1));
        // Every reader dropped its socket, so a plain (non-reuseport) bind works.
        StdUdpSocket::bind(addr).unwrap();
    }

    // N11: a reader closed while the channel is full used to drop the batch
    // it was blocked on (and every later slot of that read) after counting
    // their bytes as succeeded.
    #[tokio::test(flavor = "multi_thread")]
    async fn close_with_full_channel_delivers_held_batches() {
        let port = StdUdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let counters = Arc::new(ByteCounters::default());
        let (tx, rx) = flume::bounded(1);
        let w = UdpListenerWorker::new(
            (Uuid::now_v7(), "test".into()),
            addr,
            tx,
            &CancellationToken::new(),
            counters.clone(),
            1,
            64,
            1 << 20,
            Duration::ZERO,
            1,
            Duration::from_millis(50),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let client = StdUdpSocket::bind("127.0.0.1:0").unwrap();
        for i in 0..20u8 {
            client.send_to(&[i; 8], addr).unwrap();
        }
        // The channel holds one batch; the reader blocks on the next.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !rx.is_full() {
            assert!(Instant::now() < deadline, "reader never filled the channel");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let closing = tokio::spawn(w.close());
        // Let the reader see the cancel before anything is taken off the channel.
        tokio::time::sleep(MAX_TICK * 3).await;
        let mut delivered = 0;
        while let Ok(batch) = rx.recv_async().await {
            delivered += batch.len() as u64;
        }
        closing.await.unwrap();
        let m = counters.get();
        assert!(delivered >= 2, "only {delivered} datagrams delivered");
        assert_eq!(m.total_bytes_succeeded, delivered * 8);
        assert_eq!(m.total_bytes_dropped, 0);
        assert_eq!(m.total_bytes, m.total_bytes_succeeded);
    }

    // N10: a reader that stops on its own must take the intake down with it,
    // or its owner keeps handing out a channel nothing feeds.
    #[tokio::test]
    async fn a_dead_reader_cancels_intake() {
        let intake = CancellationToken::new();
        let (tx, _rx) = flume::bounded(16);
        let w = UdpListenerWorker::new(
            (Uuid::now_v7(), "test".into()),
            "127.0.0.1:0".parse().unwrap(),
            tx,
            &intake,
            Arc::default(),
            1,
            64,
            1 << 20,
            Duration::ZERO,
            1,
            Duration::from_millis(10),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        w.kill_readers();
        tokio::time::timeout(Duration::from_secs(2), intake.cancelled())
            .await
            .expect("intake still running after its reader died");
        w.close().await;
    }
}
