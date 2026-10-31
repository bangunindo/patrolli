// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Throughput harness for `common::UdpListenerWorker`, no Docker needed.
//!
//! Blasts loopback UDP at the listener with `sendmmsg` from several sender
//! threads (distinct source ports, so SO_REUSEPORT spreads them) and reports
//! received throughput plus kernel-side drops: the global `RcvbufErrors` delta
//! from `/proc/net/snmp` and the per-socket `drops` column of `/proc/net/udp`.
//!
//! Not an assertion. Run it in release:
//!
//! ```bash
//! cargo test --release --test bench_udp_listener -- --ignored --nocapture
//! BENCH_GBPS=0 BENCH_PKT=256 BENCH_WORKERS=4 cargo test --release --test bench_udp_listener -- --ignored --nocapture
//! ```
//!
//! Knobs (env): `BENCH_SECS` (10), `BENCH_PKT` payload bytes (512), `BENCH_GBPS`
//! offered load, 0 = unthrottled (2.0), `BENCH_SENDERS` (4), `BENCH_WORKERS`
//! listener sockets (4), `BENCH_MAX_PKT` (4096), `BENCH_RCVBUF` requested
//! receive buffer, 212992 emulates a stock `rmem_max` (32 MiB),
//! `BENCH_SPIN_US` the listener's `spin_before_block_us` (0), `BENCH_BATCH` (100),
//! `BENCH_RT_THREADS` tokio workers (4), `BENCH_HOGS` CPU-bound tokio tasks
//! that compete with the listener the way parsing does (0), `BENCH_WORK_NS`
//! per-packet busy work in the consumer (0), `BENCH_THREAD_HOGS` busy OS
//! threads competing for every core the way the processor workers do (0).

// sendmmsg and /proc/net are Linux-only.
#![cfg(target_os = "linux")]

use patrolli::common::UdpListenerWorker;
use std::net::{SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use uuid::Uuid;

fn env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn rcvbuf_errors() -> u64 {
    let snmp = std::fs::read_to_string("/proc/net/snmp").unwrap();
    let mut lines = snmp.lines().filter(|l| l.starts_with("Udp:"));
    let header: Vec<&str> = lines.next().unwrap().split_whitespace().collect();
    let values: Vec<&str> = lines.next().unwrap().split_whitespace().collect();
    let idx = header.iter().position(|h| *h == "RcvbufErrors").unwrap();
    values[idx].parse().unwrap()
}

/// Sum of the `drops` column over every socket bound to `port`.
fn socket_drops(port: u16) -> u64 {
    let udp = std::fs::read_to_string("/proc/net/udp").unwrap();
    let port_hex = format!(":{port:04X}");
    udp.lines()
        .skip(1)
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            (cols.get(1)?.ends_with(&port_hex)).then(|| cols.last()?.parse::<u64>().ok())?
        })
        .sum()
}

/// CPU ticks (utime + stime) per thread name prefix, from `/proc/self/task`.
fn thread_ticks(prefix: &str) -> u64 {
    let mut total = 0;
    for task in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
        let comm = std::fs::read_to_string(task.path().join("comm")).unwrap_or_default();
        if !comm.starts_with(prefix) {
            continue;
        }
        let stat = std::fs::read_to_string(task.path().join("stat")).unwrap_or_default();
        // Fields after the `(comm)`: state is field 3, utime 14, stime 15.
        let Some(rest) = stat.rsplit_once(") ").map(|(_, r)| r) else {
            continue;
        };
        let f: Vec<&str> = rest.split_whitespace().collect();
        total += f[11].parse::<u64>().unwrap_or(0) + f[12].parse::<u64>().unwrap_or(0);
    }
    total
}

fn spin(ns: u64) {
    if ns == 0 {
        return;
    }
    let until = Instant::now() + Duration::from_nanos(ns);
    while Instant::now() < until {
        std::hint::spin_loop();
    }
}

fn sender(
    target: SocketAddr,
    pkt: usize,
    bytes_per_sec: f64,
    stop: Arc<AtomicBool>,
    sent: Arc<AtomicU64>,
) {
    const VLEN: usize = 64;
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.connect(target).unwrap();
    let fd = sock.as_raw_fd();
    let payload = vec![b'x'; pkt];
    let mut iov: Vec<libc::iovec> = (0..VLEN)
        .map(|_| libc::iovec {
            iov_base: payload.as_ptr() as *mut _,
            iov_len: pkt,
        })
        .collect();
    let mut msgs: Vec<libc::mmsghdr> = iov
        .iter_mut()
        .map(|iov| {
            let mut hdr: libc::mmsghdr = unsafe { std::mem::zeroed() };
            hdr.msg_hdr.msg_iov = iov;
            hdr.msg_hdr.msg_iovlen = 1;
            hdr
        })
        .collect();
    let start = Instant::now();
    let mut total = 0u64;
    while !stop.load(Ordering::Relaxed) {
        if bytes_per_sec > 0.0 {
            let allowed = start.elapsed().as_secs_f64() * bytes_per_sec;
            if (total * pkt as u64) as f64 > allowed {
                std::thread::yield_now();
                continue;
            }
        }
        let n = unsafe { libc::sendmmsg(fd, msgs.as_mut_ptr(), VLEN as u32, 0) };
        if n > 0 {
            total += n as u64;
            sent.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
}

#[test]
#[ignore = "throughput benchmark: run in release with --ignored --nocapture"]
fn bench_udp_listener() {
    let secs: u64 = env("BENCH_SECS", 10);
    let pkt: usize = env("BENCH_PKT", 512);
    let gbps: f64 = env("BENCH_GBPS", 2.0);
    let senders: usize = env("BENCH_SENDERS", 4);
    let workers: usize = env("BENCH_WORKERS", 4);
    let max_pkt: usize = env("BENCH_MAX_PKT", 4096);
    let rcvbuf: usize = env("BENCH_RCVBUF", 32 << 20);
    let spin_us: u64 = env("BENCH_SPIN_US", 0);
    let batch: usize = env("BENCH_BATCH", 100);
    let rt_threads: usize = env("BENCH_RT_THREADS", 4);
    let hogs: usize = env("BENCH_HOGS", 0);
    let work_ns: u64 = env("BENCH_WORK_NS", 0);
    let thread_hogs: usize = env("BENCH_THREAD_HOGS", 0);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(rt_threads)
        .enable_all()
        .build()
        .unwrap();

    let port = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    rt.block_on(async move {
        let (tx, rx) = flume::bounded(env("BENCH_BUFCAP", 100));
        let listener = UdpListenerWorker::new(
            (Uuid::now_v7(), "bench".into()),
            addr,
            tx,
            &tokio_util::sync::CancellationToken::new(),
            Arc::default(),
            workers,
            max_pkt,
            rcvbuf,
            Duration::from_micros(spin_us),
            batch,
            Duration::from_millis(100),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();

        let recv_pkts = Arc::new(AtomicU64::new(0));
        let recv_bytes = Arc::new(AtomicU64::new(0));
        let mut consumers = Vec::new();
        for _ in 0..workers {
            let rx = rx.clone();
            let recv_pkts = recv_pkts.clone();
            let recv_bytes = recv_bytes.clone();
            consumers.push(tokio::spawn(async move {
                while let Ok(batch) = rx.recv_async().await {
                    let mut bytes = 0;
                    for (p, _) in &batch {
                        bytes += p.len() as u64;
                        spin(work_ns);
                    }
                    recv_pkts.fetch_add(batch.len() as u64, Ordering::Relaxed);
                    recv_bytes.fetch_add(bytes, Ordering::Relaxed);
                }
            }));
        }

        let stop = Arc::new(AtomicBool::new(false));
        let mut hog_handles = Vec::new();
        for _ in 0..hogs {
            let stop = stop.clone();
            // Busy for 5ms, then yield: a parse-heavy task that never awaits I/O.
            hog_handles.push(tokio::spawn(async move {
                while !stop.load(Ordering::Relaxed) {
                    spin(5_000_000);
                    tokio::task::yield_now().await;
                }
            }));
        }

        let thread_hog_handles: Vec<_> = (0..thread_hogs)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        spin(1_000_000);
                    }
                })
            })
            .collect();

        tokio::time::sleep(Duration::from_millis(200)).await;
        let rcvbuf_before = rcvbuf_errors();
        let drops_before = socket_drops(port);
        let sent = Arc::new(AtomicU64::new(0));
        let per_sender = if gbps > 0.0 {
            gbps * 1e9 / 8.0 / senders as f64
        } else {
            0.0
        };
        let send_threads: Vec<_> = (0..senders)
            .map(|_| {
                let stop = stop.clone();
                let sent = sent.clone();
                std::thread::spawn(move || sender(addr, pkt, per_sender, stop, sent))
            })
            .collect();

        let ticks_before = (thread_ticks("udp-recv"), thread_ticks("tokio-runtime"));
        let start = Instant::now();
        let mut last = (0u64, 0u64, Instant::now());
        for _ in 0..secs {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let (s, r, now) = (
                sent.load(Ordering::Relaxed),
                recv_pkts.load(Ordering::Relaxed),
                Instant::now(),
            );
            let dt = now.duration_since(last.2).as_secs_f64();
            println!(
                "  sent {:>9.0} pps  recv {:>9.0} pps  {:>6.2} Gbps  drops {}",
                (s - last.0) as f64 / dt,
                (r - last.1) as f64 / dt,
                (r - last.1) as f64 * pkt as f64 * 8.0 / dt / 1e9,
                socket_drops(port) - drops_before,
            );
            last = (s, r, now);
        }
        stop.store(true, Ordering::Relaxed);
        for t in send_threads.into_iter().chain(thread_hog_handles) {
            t.join().unwrap();
        }
        let elapsed = start.elapsed().as_secs_f64();
        let ticks = (
            thread_ticks("udp-recv") - ticks_before.0,
            thread_ticks("tokio-runtime") - ticks_before.1,
        );
        // Let in-flight batches (batch_timeout 100ms) drain.
        tokio::time::sleep(Duration::from_millis(500)).await;

        let sent = sent.load(Ordering::Relaxed);
        let recv = recv_pkts.load(Ordering::Relaxed);
        let drops = socket_drops(port) - drops_before;
        let rcvbuf = rcvbuf_errors() - rcvbuf_before;
        let m = listener.get_metrics();
        println!(
            "\nRESULT pkt={pkt} gbps_target={gbps} senders={senders} workers={workers} batch={batch} \
             rt_threads={rt_threads} hogs={hogs} thread_hogs={thread_hogs} work_ns={work_ns}"
        );
        println!(
            "  sent {sent} ({:.0} pps, {:.2} Gbps)  recv {recv} ({:.0} pps, {:.2} Gbps)",
            sent as f64 / elapsed,
            sent as f64 * pkt as f64 * 8.0 / elapsed / 1e9,
            recv as f64 / elapsed,
            recv as f64 * pkt as f64 * 8.0 / elapsed / 1e9,
        );
        println!(
            "  socket drops {drops}  RcvbufErrors {rcvbuf}  loss {:.3}%  listener bytes ok {}",
            100.0 * (sent.saturating_sub(recv)) as f64 / sent.max(1) as f64,
            m.total_bytes_succeeded,
        );
        let hwm = std::fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .find(|l| l.starts_with("VmHWM:"))
            .map(|l| l.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() / 1024)
            .unwrap_or(0);
        // USER_HZ is 100 on every Linux build that matters.
        println!(
            "  cpu: udp-recv threads {:.0}%  tokio workers {:.0}%  peak rss {hwm} MiB",
            ticks.0 as f64 / elapsed,
            ticks.1 as f64 / elapsed,
        );

        listener.close().await;
        for c in consumers {
            c.await.unwrap();
        }
        for h in hog_handles {
            h.await.unwrap();
        }
    });
}
