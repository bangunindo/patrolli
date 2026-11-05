// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Throughput harness for the syslog listener, no Docker needed.
//!
//! Drives a real `listener::WorkerGroup` with one syslog listener: TCP clients
//! (or `sendmmsg` UDP senders) blast fixed-length frames at it, the listener
//! parses and annotates them, and a drain thread counts what reaches the
//! processor channel. The listener is closed while traffic still flows, so the
//! run also measures shutdown latency under load.
//!
//! Not an assertion. Run it in release:
//!
//! ```bash
//! cargo test --release --test bench_syslog_listener -- --ignored --nocapture
//! BENCH_CONNS=64 cargo test --release --test bench_syslog_listener -- --ignored --nocapture
//! BENCH_CONNS=2000 BENCH_RATE=200000 cargo test --release --test bench_syslog_listener -- --ignored --nocapture
//! BENCH_MODE=udp BENCH_CONNS=4 cargo test --release --test bench_syslog_listener -- --ignored --nocapture
//! ```
//!
//! Knobs (env): `BENCH_MODE` tcp|udp (tcp), `BENCH_SECS` measured seconds (10),
//! `BENCH_CONNS` TCP connections or UDP senders (8), `BENCH_CLIENT_THREADS` TCP
//! client threads the connections are spread over (min(conns, 16)),
//! `BENCH_RATE` total offered msg/s, 0 = unthrottled (0), `BENCH_MSG` message
//! bytes before framing (200), `BENCH_FRAMING` lf|octet (lf), `BENCH_WORKERS`
//! listener `worker_count` (8), `BENCH_BATCH` (1000), `BENCH_TIMEOUT_MS` batch
//! timeout (1000), `BENCH_RT_THREADS` tokio workers (8), `BENCH_UDP_SOCKETS`
//! reader sockets (4), `BENCH_BUFCAP` UDP `buffer_cap` (100).
//!
//! CPU is the listener's own: tokio workers plus `udp-recv` reader threads,
//! from `/proc/self/task`. Client and drain threads are excluded, so on
//! loopback, where clients can saturate first, `cpu µs/msg` is the number to
//! compare.

// sendmmsg and /proc/net are Linux-only.
#![cfg(target_os = "linux")]

use arc_swap::ArcSwap;
use patrolli::common::internal_metrics;
use patrolli::listener::WorkerGroup;
use patrolli::payload::ProcessorPayload;
use patrolli::processor::PROCESSOR_CHANNEL_NAME;
use patrolli::{cache, chan, config};
use std::io::Write;
use std::net::{SocketAddr, TcpStream, UdpSocket};
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

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
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

fn listener_ticks() -> u64 {
    thread_ticks("tokio-rt-worker") + thread_ticks("udp-recv")
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

/// One syslog message of exactly `len` bytes.
fn message(len: usize) -> Vec<u8> {
    let mut m = b"<13>1 2026-10-02T00:00:00Z bench-host bench-app 1 - - ".to_vec();
    while m.len() < len {
        m.push(b'x');
    }
    m
}

/// One framed message.
fn frame(msg: &[u8], octet: bool) -> Vec<u8> {
    let mut f = Vec::with_capacity(msg.len() + 8);
    if octet {
        f.extend_from_slice(format!("{} ", msg.len()).as_bytes());
        f.extend_from_slice(msg);
    } else {
        f.extend_from_slice(msg);
        f.push(b'\n');
    }
    f
}

/// A cache where every record of `listener_id` belongs to one item, so
/// annotation sends it on instead of dropping it.
fn item_for(listener_id: Uuid) -> cache::Cache {
    let cache = cache::Cache::new();
    let host_id = Uuid::now_v7();
    cache.host.write().insert(config::host::Host {
        id: host_id,
        name: "host".into(),
        timezone: None,
    });
    let item = config::item::Item::new(
        Uuid::now_v7(),
        "syslog-item".into(),
        None,
        host_id,
        config::item::Schedule::Never,
        Some(listener_id),
        Duration::from_secs(1),
        vec![Uuid::now_v7()],
        config::item::Config::Listener(config::item::Listener {
            match_strategy: config::item::ListenerItemMatchStrategy::None,
        }),
    );
    cache.item.write().insert(item.clone());
    cache.listener.write().insert_item(item, None);
    cache
}

/// Writes frames round-robin over `conns`, `chunk` frames per write, paced to
/// `rate` msg/s (0 = unthrottled). Returns on `stop` or once every connection
/// is gone.
fn tcp_client(
    mut conns: Vec<TcpStream>,
    frame: Vec<u8>,
    chunk: usize,
    rate: f64,
    stop: Arc<AtomicBool>,
    sent: Arc<AtomicU64>,
) {
    let buf = frame.repeat(chunk);
    let start = Instant::now();
    let mut total = 0u64;
    let mut i = 0;
    while !stop.load(Ordering::Relaxed) && !conns.is_empty() {
        if rate > 0.0 {
            let allowed = start.elapsed().as_secs_f64() * rate;
            if total as f64 >= allowed {
                std::thread::sleep(Duration::from_micros(200));
                continue;
            }
        }
        i %= conns.len();
        // The listener closing the stream ends that connection.
        if conns[i].write_all(&buf).is_err() {
            conns.swap_remove(i);
            continue;
        }
        total += chunk as u64;
        sent.fetch_add(chunk as u64, Ordering::Relaxed);
        i += 1;
    }
}

fn udp_sender(
    target: SocketAddr,
    msg: Vec<u8>,
    stop: Arc<AtomicBool>,
    sent: Arc<AtomicU64>,
    rate: f64,
) {
    const VLEN: usize = 64;
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.connect(target).unwrap();
    let fd = sock.as_raw_fd();
    let mut iov: Vec<libc::iovec> = (0..VLEN)
        .map(|_| libc::iovec {
            iov_base: msg.as_ptr() as *mut _,
            iov_len: msg.len(),
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
        if rate > 0.0 && total as f64 >= start.elapsed().as_secs_f64() * rate {
            std::thread::sleep(Duration::from_micros(200));
            continue;
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
fn bench_syslog_listener() {
    let mode = env_str("BENCH_MODE", "tcp");
    let udp = mode == "udp";
    let secs: u64 = env("BENCH_SECS", 10);
    let conns: usize = env("BENCH_CONNS", 8);
    let client_threads: usize = env("BENCH_CLIENT_THREADS", conns.min(16)).max(1);
    let rate: f64 = env("BENCH_RATE", 0.0);
    let msg_len: usize = env("BENCH_MSG", 200);
    let octet = env_str("BENCH_FRAMING", "lf") == "octet";
    let workers: usize = env("BENCH_WORKERS", 8);
    let batch_size: usize = env("BENCH_BATCH", 1000);
    let batch_timeout = Duration::from_millis(env("BENCH_TIMEOUT_MS", 1000));
    let rt_threads: usize = env("BENCH_RT_THREADS", 8);
    let udp_sockets: usize = env("BENCH_UDP_SOCKETS", 4);
    let buffer_cap: usize = env("BENCH_BUFCAP", 100);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(rt_threads)
        .enable_all()
        .build()
        .unwrap();

    let addr: SocketAddr = if udp {
        UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    } else {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    };
    let msg = message(msg_len);
    let msg_len = msg.len();

    rt.block_on(async move {
        let (admin, sender) = chan::new::<&'static str, ProcessorPayload>(
            Duration::from_secs(3600),
            Arc::new(ArcSwap::from_pointee((
                Instant::now(),
                internal_metrics::Channel::default(),
            ))),
        );
        let processor_rx = admin
            .get_receiver(
                &PROCESSOR_CHANNEL_NAME,
                config::Channel::Memory(config::MemoryChannel {
                    non_blocking: false,
                    channel_buffer_size: 64,
                }),
            )
            .await
            .unwrap();
        let rows = Arc::new(AtomicU64::new(0));
        let payloads = Arc::new(AtomicU64::new(0));
        {
            let (rows, payloads) = (rows.clone(), payloads.clone());
            std::thread::Builder::new()
                .name("bench-drain".into())
                .spawn(move || {
                    while let Ok(pl) = processor_rx.recv() {
                        rows.fetch_add(pl.payload.num_rows() as u64, Ordering::Relaxed);
                        payloads.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .unwrap();
        }

        let listener_id = Uuid::now_v7();
        let transport = config::listener::SyslogListener {
            udp: udp.then(|| config::listener::UdpListener {
                address: addr,
                max_packet_size: 4096,
                buffer_cap,
                recv_buffer_size: 32 << 20,
                spin_before_block_us: 0,
                worker_count: udp_sockets,
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
        let conf = config::listener::Listener::new(
            listener_id,
            "syslog-bench".into(),
            workers,
            config::Batch {
                batch_size,
                batch_timeout,
            },
            config::listener::Config::Syslog(Box::new(config::listener::Syslog {
                listener: transport,
                cise: Default::default(),
                cef: Default::default(),
                cisco: Default::default(),
            })),
            false,
        );
        let group = WorkerGroup::new(sender, None, item_for(listener_id), None);
        group.add(conf).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let stop = Arc::new(AtomicBool::new(false));
        let sent = Arc::new(AtomicU64::new(0));
        let drops_before = if udp { socket_drops(addr.port()) } else { 0 };
        let clients: Vec<_> = if udp {
            let per = rate / conns as f64;
            (0..conns)
                .map(|i| {
                    let (stop, sent, msg) = (stop.clone(), sent.clone(), msg.clone());
                    std::thread::Builder::new()
                        .name(format!("bench-client-{i}"))
                        .spawn(move || udp_sender(addr, msg, stop, sent, per))
                        .unwrap()
                })
                .collect()
        } else {
            let frame = frame(&msg, octet);
            // ~64 KiB per write unthrottled; small writes when paced, so slow
            // streams look like slow streams.
            let chunk = if rate > 0.0 {
                1
            } else {
                (64 * 1024 / frame.len()).max(1)
            };
            let mut groups: Vec<Vec<TcpStream>> = (0..client_threads).map(|_| Vec::new()).collect();
            for i in 0..conns {
                let s = TcpStream::connect(addr).expect("connect");
                s.set_nodelay(true).unwrap();
                groups[i % client_threads].push(s);
            }
            let per = rate / client_threads as f64;
            groups
                .into_iter()
                .enumerate()
                .map(|(i, conns)| {
                    let (stop, sent, frame) = (stop.clone(), sent.clone(), frame.clone());
                    std::thread::Builder::new()
                        .name(format!("bench-client-{i}"))
                        .spawn(move || tcp_client(conns, frame, chunk, per, stop, sent))
                        .unwrap()
                })
                .collect()
        };

        // Warm up, then measure.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let ticks0 = listener_ticks();
        let (sent0, rows0, payloads0) = (
            sent.load(Ordering::Relaxed),
            rows.load(Ordering::Relaxed),
            payloads.load(Ordering::Relaxed),
        );
        let start = Instant::now();
        let mut last = (sent0, rows0, Instant::now());
        for _ in 0..secs {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let (s, r, now) = (
                sent.load(Ordering::Relaxed),
                rows.load(Ordering::Relaxed),
                Instant::now(),
            );
            let dt = now.duration_since(last.2).as_secs_f64();
            println!(
                "  sent {:>10.0} msg/s  delivered {:>10.0} msg/s",
                (s - last.0) as f64 / dt,
                (r - last.1) as f64 / dt,
            );
            last = (s, r, now);
        }
        let elapsed = start.elapsed().as_secs_f64();
        let ticks = listener_ticks() - ticks0;
        let sent_n = sent.load(Ordering::Relaxed) - sent0;
        let rows_n = rows.load(Ordering::Relaxed) - rows0;
        let payloads_n = payloads.load(Ordering::Relaxed) - payloads0;
        let m = group
            .metrics()
            .await
            .into_iter()
            .find(|m| m.id == listener_id)
            .unwrap();

        // Read before the close, which takes the sockets away.
        let drops = if udp {
            socket_drops(addr.port()) - drops_before
        } else {
            0
        };

        // Close while traffic still flows.
        let close_start = Instant::now();
        let closed =
            tokio::time::timeout(Duration::from_secs(30), group.close_listener(listener_id)).await;
        let close_time = close_start.elapsed();
        stop.store(true, Ordering::Relaxed);

        println!(
            "\nRESULT mode={mode} conns={conns} rate={rate} msg={msg_len} framing={} workers={workers} \
             batch={batch_size} timeout_ms={} rt_threads={rt_threads}",
            if octet { "octet" } else { "lf" },
            batch_timeout.as_millis(),
        );
        println!(
            "  offered {:.0} msg/s  delivered {:.0} msg/s ({:.1} MB/s)",
            sent_n as f64 / elapsed,
            rows_n as f64 / elapsed,
            rows_n as f64 * msg_len as f64 / elapsed / 1e6,
        );
        // USER_HZ is 100 on every Linux build that matters.
        let cpu_us = ticks as f64 * 10_000.0;
        println!(
            "  cpu {:.0}%  cpu {:.3} µs/msg  rows/payload {:.0}",
            ticks as f64 / elapsed,
            cpu_us / rows_n.max(1) as f64,
            rows_n as f64 / payloads_n.max(1) as f64,
        );
        println!(
            "  transport bytes dropped {}  records dropped {}  socket drops {drops}",
            m.total_bytes_dropped, m.total_records_dropped,
        );
        match closed {
            Ok(res) => {
                res.unwrap();
                println!("  close under load {:.1} ms", close_time.as_secs_f64() * 1e3);
            }
            Err(_) => {
                println!("  close under load HUNG (>30 s)");
                // Neither the hung listener nor its blocked clients can be
                // torn down.
                std::process::exit(0);
            }
        }

        // Clients see their connections closed and return. The drain thread
        // is left detached: it ends with the process.
        for c in clients {
            c.join().unwrap();
        }
        group.close(patrolli::common::shutdown::Deadline::NONE).await;
    });
}
