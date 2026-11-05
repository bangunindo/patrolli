// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! NetFlow → `file` sink (`path: "null"`) throughput harness, no Docker needed.
//!
//! Replays the UDP payloads of a pcap through a real `patrolli` binary in local
//! mode, each exporter from its own loopback address (`127.0.0.<n>`) and its
//! own sender thread, so every exporter's packets stay in capture order, the
//! way its templates and options records need them. The default capture is
//! the Cisco FNF one `netflow_parser` ships (`pcaps/IPFIX.pcap`, two
//! exporters): it carries options templates (a sampler table among them) and
//! data templates that name a sampler, so it exercises the sampling-rate path,
//! unlike a single-template fixture.
//!
//! The capture is sent once at a gentle pace to load templates, then looped
//! for `BENCH_SECS`. The numbers are the deltas of the binary's own cumulative
//! internal metrics over that window, plus its process CPU time and the
//! kernel's `RcvbufErrors`. `engine ns/datagram` is the parse cost alone (the
//! listener's `engine_time_spent`), and per datagram rather than per record
//! because the input is fixed while the records a build makes of it are not.
//! Compare builds at an offered load they both keep up with (no
//! `RcvbufErrors`): past it the kernel drops datagrams at random, templates
//! and options among them, so two runs no longer parse the same traffic.
//!
//! Not an assertion. Run it in release, and to compare two builds point
//! `PATROLLI_BIN` at each:
//!
//! ```bash
//! cargo test --release --test bench_netflow_replay -- --ignored --nocapture
//! PATROLLI_BIN=/path/to/old/patrolli BENCH_PATROLLI_CPUS=0-3 BENCH_SENDER_CPUS=8-15 \
//!     cargo test --release --test bench_netflow_replay -- --ignored --nocapture
//! ```
//!
//! Knobs (env): `PATROLLI_BIN` (the binary cargo built for this test),
//! `BENCH_PCAP` (netflow_parser's `IPFIX.pcap` under `CARGO_HOME`),
//! `BENCH_SECS` (10), `BENCH_PPS` total offered datagrams/s, 0 = unthrottled
//! (0), `BENCH_WORKERS` the listener's `worker_count` (1), `BENCH_PATROLLI_CPUS`
//! / `BENCH_SENDER_CPUS` CPU lists like `0-7,16-23` to pin the binary and the
//! senders apart (unpinned).

// sendmmsg, sched_setaffinity and /proc are Linux-only.
#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const LISTENER_ID: &str = "019b77b3-df79-70a2-ab01-b43b2cf723a6";
const SINK_ID: &str = "019b6e6e-7e75-70a2-b0ba-d88cb64a9d4b";
const STARTUP_LIMIT: Duration = Duration::from_secs(60);
/// The internal metrics interval, and so how stale a snapshot can be.
const METRICS_INTERVAL: Duration = Duration::from_secs(1);

fn env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// `0-3,8,16-19` → the CPUs it names.
fn cpu_list(spec: &str) -> Vec<usize> {
    spec.split(',')
        .filter(|s| !s.is_empty())
        .flat_map(|part| match part.split_once('-') {
            Some((a, b)) => a.trim().parse().unwrap()..=b.trim().parse().unwrap(),
            None => {
                let n = part.trim().parse().unwrap();
                n..=n
            }
        })
        .collect()
}

fn set_affinity(cpus: &[usize]) -> std::io::Result<()> {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        for &cpu in cpus {
            libc::CPU_SET(cpu, &mut set);
        }
        if libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &set) != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn default_pcap() -> PathBuf {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cargo"));
    let registry = cargo_home.join("registry/src");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&registry)
        .unwrap_or_else(|e| panic!("read {}: {e}; set BENCH_PCAP", registry.display()))
        .flatten()
        .flat_map(|index| {
            std::fs::read_dir(index.path())
                .into_iter()
                .flatten()
                .flatten()
        })
        .filter(|krate| {
            krate
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("netflow_parser-"))
        })
        .map(|krate| krate.path().join("pcaps/IPFIX.pcap"))
        .filter(|p| p.exists())
        .collect();
    found.sort();
    found
        .pop()
        .expect("no netflow_parser-*/pcaps/IPFIX.pcap in the cargo registry; set BENCH_PCAP")
}

/// The UDP payloads of a classic pcap (Ethernet with optional VLAN tags,
/// Linux cooked, or raw IP), with their source address, in capture order.
fn pcap_udp(path: &Path) -> Vec<(IpAddr, Vec<u8>)> {
    let d = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let le = match &d[..4] {
        [0xd4, 0xc3, 0xb2, 0xa1] | [0x4d, 0x3c, 0xb2, 0xa1] => true,
        [0xa1, 0xb2, 0xc3, 0xd4] | [0xa1, 0xb2, 0x3c, 0x4d] => false,
        _ => panic!(
            "{}: not a classic pcap (pcapng is not supported)",
            path.display()
        ),
    };
    let u32_at = |b: &[u8], o: usize| {
        let v: [u8; 4] = b[o..o + 4].try_into().unwrap();
        if le {
            u32::from_le_bytes(v)
        } else {
            u32::from_be_bytes(v)
        }
    };
    let be16 = |b: &[u8], o: usize| u16::from_be_bytes([b[o], b[o + 1]]);
    let linktype = u32_at(&d, 20);
    let mut out = Vec::new();
    let mut off = 24;
    while off + 16 <= d.len() {
        let incl = u32_at(&d, off + 8) as usize;
        let pkt = &d[off + 16..(off + 16 + incl).min(d.len())];
        off += 16 + incl;
        let (mut ethertype, mut p) = match linktype {
            1 => (be16(pkt, 12), 14),
            113 => (be16(pkt, 14), 16),
            101 => (if pkt[0] >> 4 == 4 { 0x0800 } else { 0x86dd }, 0),
            other => panic!("unsupported pcap link type {other}"),
        };
        while ethertype == 0x8100 || ethertype == 0x88a8 {
            ethertype = be16(pkt, p + 2);
            p += 4;
        }
        let (proto, src) = match ethertype {
            0x0800 => {
                let ihl = (pkt[p] & 0x0f) as usize * 4;
                let src: [u8; 4] = pkt[p + 12..p + 16].try_into().unwrap();
                p += ihl;
                (pkt[p - ihl + 9], IpAddr::from(src))
            }
            0x86dd => {
                let src: [u8; 16] = pkt[p + 8..p + 24].try_into().unwrap();
                p += 40;
                (pkt[p - 40 + 6], IpAddr::from(src))
            }
            _ => continue,
        };
        if proto != 17 || pkt.len() < p + 8 {
            continue;
        }
        let len = (be16(pkt, p + 4) as usize).min(pkt.len() - p);
        out.push((src, pkt[p + 8..p + len].to_vec()));
    }
    out
}

fn rcvbuf_errors() -> u64 {
    let snmp = std::fs::read_to_string("/proc/net/snmp").unwrap();
    let mut lines = snmp.lines().filter(|l| l.starts_with("Udp:"));
    let header: Vec<&str> = lines.next().unwrap().split_whitespace().collect();
    let values: Vec<&str> = lines.next().unwrap().split_whitespace().collect();
    let idx = header.iter().position(|h| *h == "RcvbufErrors").unwrap();
    values[idx].parse().unwrap()
}

/// utime + stime of `pid`, in seconds.
fn cpu_seconds(pid: u32) -> f64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    // the command name may hold spaces; the fields after it do not
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    let ticks: u64 = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();
    ticks as f64 / unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn config(port: u16, workers: usize) -> String {
    format!(
        r#"global:
  logger:
    level: info
    disable_color: true
  mib:
    disabled: true
  disk_channel:
    data_dir: ./data
listeners:
  - type: netflow
    id: {LISTENER_ID}
    name: netflow-1
    worker_count: {workers}
    batch_size: 1000
    config:
      listener:
        address: 127.0.0.1:{port}
        max_packet_size: 9000
  - type: internal_metrics
    id: 019b77b3-df79-70a2-ab01-f43b2cf723a9
    name: internal-metrics-1
    worker_count: 1
    config:
      report_throughput: false
      interval: {interval}
hosts:
  - id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    name: host-1
processors:
  - id: 019c98c8-5512-70a2-906e-7ccd03618702
    name: metrics
    type: "null"
  - id: 019c98c8-5512-70a2-906e-7ccd03618703
    name: flows
    type: "null"
items:
  - type: listener
    name: netflow-listener
    id: 019bd3f0-f429-70a2-8602-508e4a18f8b9
    listener_id: {LISTENER_ID}
    host_id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    config:
      match_strategy:
        method: none
    processors:
      - 019c98c8-5512-70a2-906e-7ccd03618703
  - type: listener
    name: internal-metrics-1
    id: 019b914b-7234-70a2-af38-89569b7c1b0f
    listener_id: 019b77b3-df79-70a2-ab01-f43b2cf723a9
    host_id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    config:
      match_strategy:
        method: none
    processors:
      - 019c98c8-5512-70a2-906e-7ccd03618702
sinks:
  - type: file
    name: internal-metrics
    id: 019b6e6e-7e75-70a2-b0ba-d88cb64a9d4a
    worker_count: 1
    inputs:
      - 019c98c8-5512-70a2-906e-7ccd03618702
    config:
      format: json_nested_denormalized
      path: metrics.jsonl
  - type: file
    name: netflow-sink
    id: {SINK_ID}
    worker_count: 1
    inputs:
      - 019c98c8-5512-70a2-906e-7ccd03618703
    config:
      format: proto_otlp
      path: "null"
"#,
        interval = METRICS_INTERVAL.as_millis(),
    )
}

/// The cumulative counters this bench reads, from one internal metrics report.
#[derive(Clone, Copy, Default, Debug)]
struct Snapshot {
    items: f64,
    records: f64,
    records_dropped: f64,
    engine_time: f64,
    sink_records: f64,
}

impl std::ops::Sub for Snapshot {
    type Output = Snapshot;
    fn sub(self, o: Snapshot) -> Snapshot {
        Snapshot {
            items: self.items - o.items,
            records: self.records - o.records,
            records_dropped: self.records_dropped - o.records_dropped,
            engine_time: self.engine_time - o.engine_time,
            sink_records: self.sink_records - o.sink_records,
        }
    }
}

/// The latest report in `metrics.jsonl`, once it holds one written after `after`.
fn snapshot_after(dir: &Path, after: Instant) -> Snapshot {
    // a report is written every interval; wait two so the one read postdates `after`
    let ready = after + 2 * METRICS_INTERVAL + Duration::from_millis(200);
    std::thread::sleep(ready.saturating_duration_since(Instant::now()));
    let text = std::fs::read_to_string(dir.join("metrics.jsonl")).unwrap();
    let last = text.lines().last().expect("no internal metrics report yet");
    let report: serde_json::Value = serde_json::from_str(last).unwrap();
    let attrs = &report["record"]["attributes"];
    let find = |kind: &str, id: &str| {
        attrs[kind]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap_or_else(|| panic!("no {kind} {id} in {last}"))
            .clone()
    };
    let listener = find("listener", LISTENER_ID);
    let sink = find("sink", SINK_ID);
    let f = |v: &serde_json::Value, k: &str| v[k].as_f64().unwrap_or(0.0);
    Snapshot {
        items: f(&listener, "total_items"),
        records: f(&listener, "total_records"),
        records_dropped: f(&listener, "total_records_dropped"),
        // a total report gives durations in nanoseconds
        engine_time: f(&listener, "engine_time_spent") / 1e9,
        sink_records: f(&sink, "total_records_sent"),
    }
}

/// Sends `packets` to `target` from `src` in order, looping until `stop`, at
/// `pps` (0 = as fast as `sendmmsg` goes). `once` sends them a single time.
fn sender(
    src: IpAddr,
    target: SocketAddr,
    packets: Arc<Vec<Vec<u8>>>,
    pps: f64,
    once: bool,
    stop: Arc<AtomicBool>,
    sent: Arc<AtomicU64>,
) {
    const VLEN: usize = 64;
    let sock = UdpSocket::bind(SocketAddr::new(src, 0)).unwrap();
    sock.connect(target).unwrap();
    let fd = sock.as_raw_fd();
    let start = Instant::now();
    let mut total = 0usize;
    'outer: loop {
        let n = if once {
            VLEN.min(packets.len() - total)
        } else {
            VLEN
        };
        if n == 0 || stop.load(Ordering::Relaxed) {
            break;
        }
        let mut iov: Vec<libc::iovec> = (0..n)
            .map(|i| {
                let p = &packets[(total + i) % packets.len()];
                libc::iovec {
                    iov_base: p.as_ptr() as *mut _,
                    iov_len: p.len(),
                }
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
        let mut done = 0;
        while done < n {
            let r = unsafe { libc::sendmmsg(fd, msgs[done..].as_mut_ptr(), (n - done) as u32, 0) };
            if r < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::ENOBUFS)
                    || err.kind() == std::io::ErrorKind::WouldBlock
                {
                    std::thread::yield_now();
                    continue;
                }
                panic!("sendmmsg: {err}");
            }
            done += r as usize;
        }
        total += n;
        sent.fetch_add(n as u64, Ordering::Relaxed);
        if pps > 0.0 {
            let due = start + Duration::from_secs_f64(total as f64 / pps);
            while Instant::now() < due {
                if stop.load(Ordering::Relaxed) {
                    break 'outer;
                }
                std::thread::sleep((due - Instant::now()).min(Duration::from_millis(5)));
            }
        }
    }
}

/// One sender thread per exporter; returns the datagrams sent.
fn replay(
    exporters: &[(IpAddr, Arc<Vec<Vec<u8>>>)],
    target: SocketAddr,
    pps: f64,
    once: bool,
    duration: Option<Duration>,
    sender_cpus: &[usize],
) -> u64 {
    let total_packets: usize = exporters.iter().map(|(_, p)| p.len()).sum();
    let stop = Arc::new(AtomicBool::new(false));
    let sent = Arc::new(AtomicU64::new(0));
    let handles: Vec<_> = exporters
        .iter()
        .enumerate()
        .map(|(i, (src, packets))| {
            // each exporter's share of the offered load, by its share of the capture
            let share = if pps > 0.0 {
                pps * packets.len() as f64 / total_packets as f64
            } else {
                0.0
            };
            let (src, packets, stop, sent) = (*src, packets.clone(), stop.clone(), sent.clone());
            let cpu = (!sender_cpus.is_empty()).then(|| sender_cpus[i % sender_cpus.len()]);
            std::thread::spawn(move || {
                if let Some(cpu) = cpu {
                    set_affinity(&[cpu]).unwrap();
                }
                sender(src, target, packets, share, once, stop, sent)
            })
        })
        .collect();
    if let Some(duration) = duration {
        std::thread::sleep(duration);
        stop.store(true, Ordering::Relaxed);
    }
    for h in handles {
        h.join().unwrap();
    }
    sent.load(Ordering::Relaxed)
}

/// Kills the binary when the bench panics.
struct Patrolli(Child);

impl Drop for Patrolli {
    fn drop(&mut self) {
        unsafe { libc::kill(self.0.id() as i32, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.0.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_patrolli(bin: &str, dir: &Path, cpus: Vec<usize>) -> Patrolli {
    let mut cmd = Command::new(bin);
    cmd.arg("--config")
        .arg("config.yaml")
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if !cpus.is_empty() {
        unsafe {
            cmd.pre_exec(move || set_affinity(&cpus));
        }
    }
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    // keep draining stderr for the whole run so the binary never blocks on it
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.contains("udp listening") {
                let _ = tx.send(());
            }
            if line.contains("ERROR") || line.contains("WARN") {
                eprintln!("patrolli: {line}");
            }
        }
    });
    let patrolli = Patrolli(child);
    rx.recv_timeout(STARTUP_LIMIT)
        .expect("patrolli never logged `udp listening`");
    patrolli
}

#[test]
#[ignore = "throughput benchmark: run in release with --ignored --nocapture"]
fn bench_netflow_replay() {
    let bin =
        std::env::var("PATROLLI_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_patrolli").into());
    let pcap = std::env::var_os("BENCH_PCAP")
        .map(PathBuf::from)
        .unwrap_or_else(default_pcap);
    let secs: u64 = env("BENCH_SECS", 10);
    let pps: f64 = env("BENCH_PPS", 0.0);
    let workers: usize = env("BENCH_WORKERS", 1);
    let patrolli_cpus = cpu_list(&env("BENCH_PATROLLI_CPUS", String::new()));
    let sender_cpus = cpu_list(&env("BENCH_SENDER_CPUS", String::new()));

    // each exporter gets a loopback address of its own, in order of appearance
    let mut by_src: Vec<(IpAddr, Vec<Vec<u8>>)> = Vec::new();
    let mut index: HashMap<IpAddr, usize> = HashMap::new();
    for (src, payload) in pcap_udp(&pcap) {
        let i = *index.entry(src).or_insert_with(|| {
            by_src.push((
                IpAddr::V4(Ipv4Addr::new(127, 0, 0, by_src.len() as u8 + 1)),
                Vec::new(),
            ));
            by_src.len() - 1
        });
        by_src[i].1.push(payload);
    }
    let exporters: Vec<(IpAddr, Arc<Vec<Vec<u8>>>)> = by_src
        .into_iter()
        .map(|(ip, packets)| (ip, Arc::new(packets)))
        .collect();
    let capture_packets: usize = exporters.iter().map(|(_, p)| p.len()).sum();
    println!(
        "pcap {}: {capture_packets} datagrams from {} exporters",
        pcap.display(),
        exporters.len()
    );

    let dir = tempfile::tempdir().unwrap();
    let port = free_udp_port();
    std::fs::write(dir.path().join("config.yaml"), config(port, workers)).unwrap();
    let patrolli = start_patrolli(&bin, dir.path(), patrolli_cpus);
    let pid = patrolli.0.id();
    let target = SocketAddr::from(([127, 0, 0, 1], port));

    // load every exporter's templates and options without dropping any
    replay(&exporters, target, 20_000.0, true, None, &sender_cpus);
    let primed = Instant::now();
    let before = snapshot_after(dir.path(), primed);
    assert!(
        before.records > 0.0,
        "the priming pass produced no records: {before:?}"
    );

    let rcvbuf_before = rcvbuf_errors();
    let cpu_before = cpu_seconds(pid);
    let t0 = Instant::now();
    let sent = replay(
        &exporters,
        target,
        pps,
        false,
        Some(Duration::from_secs(secs)),
        &sender_cpus,
    );
    let send_secs = t0.elapsed().as_secs_f64();
    // let the binary finish what is queued, then read the counters
    let after = snapshot_after(dir.path(), Instant::now());
    let cpu = cpu_seconds(pid) - cpu_before;
    let wall = t0.elapsed().as_secs_f64();
    let rcvbuf = rcvbuf_errors() - rcvbuf_before;
    drop(patrolli);

    let d = after - before;
    let per_dgram = |secs: f64| 1e9 * secs / d.items.max(1.0);
    println!(
        "offered {:.0} datagrams/s ({sent} in {send_secs:.1}s); kernel RcvbufErrors {rcvbuf} ({:.2}%)",
        sent as f64 / send_secs,
        100.0 * rcvbuf as f64 / sent.max(1) as f64,
    );
    println!(
        "listener: {:.0} datagrams ({:.0}/s), {:.0} records ({:.0}/s), {:.0} records dropped",
        d.items,
        d.items / send_secs,
        d.records,
        d.records / send_secs,
        d.records_dropped,
    );
    println!(
        "engine: {:.0} ns/datagram, {:.1} ns/record",
        per_dgram(d.engine_time),
        1e9 * d.engine_time / d.records.max(1.0),
    );
    println!(
        "sink: {:.0} records sent; process CPU {cpu:.2}s over {wall:.1}s, {:.0} ns/datagram",
        d.sink_records,
        per_dgram(cpu),
    );
    println!(
        "{}",
        serde_json::json!({
            "bin": bin,
            "sent": sent,
            "send_secs": send_secs,
            "rcvbuf_errors": rcvbuf,
            "datagrams": d.items,
            "records": d.records,
            "records_per_sec": d.records / send_secs,
            "records_dropped": d.records_dropped,
            "engine_ns_per_datagram": per_dgram(d.engine_time),
            "engine_ns_per_record": 1e9 * d.engine_time / d.records.max(1.0),
            "sink_records": d.sink_records,
            "cpu_secs": cpu,
            "cpu_ns_per_datagram": per_dgram(cpu),
        })
    );
}
