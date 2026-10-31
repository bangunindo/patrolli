// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// Throughput harness for the disk-backed `chan` topic, with the block device's
// own counters sampled alongside so the channel's rate can be set against raw
// disk throughput / IOPS (compare with a `fio` baseline on the same device).
//
// One scenario per process: fjall's database is a process-global, so a fresh
// process (and a fresh `DCB_DIR`) per run is the only clean state.
//
// Phases:
//   spill  — producers send, nobody receives: every payload goes to disk.
//   drain  — the backlog is made cold (fsync + POSIX_FADV_DONTNEED on every
//            file), then consumers receive until the topic is empty.
//   live   — producers and a consumer run together (`DCB_MODE=live`); the
//            consumer can be rate-limited so a backlog builds and both the
//            write and the read side hit the disk.
//
// It writes real data, so it is `#[ignore]` and bounded three ways: by time,
// by the topic's on-disk size, and by the filesystem's free space.
//
//   DCB_DIR=target/diskbench/run cargo test --release --test bench_disk_channel \
//       -- --ignored --nocapture
//
// Env (defaults in brackets):
//   DCB_DIR            fjall data dir, must be on the disk under test   [required]
//   DCB_MODE           spill_drain | live                               [spill_drain]
//   DCB_ROWS           log rows per payload                             [1000]
//   DCB_COMP           none | lz4 | zstd                                [zstd]
//   DCB_TRANSPORT      1: topic `transport_optimized`                   [0]
//   DCB_WORKERS        topic `worker_count` (bridge tasks per side)      [4]
//   DCB_BUFFER         topic `channel_buffer_size`                      [10]
//   DCB_PRODUCERS      producer threads                                 [4]
//   DCB_CONSUMERS      consumer tasks                                   [4]
//   DCB_SECS           spill / live duration cap                        [60]
//   DCB_MAX_GB         stop once the topic holds this much on disk      [30]
//   DCB_MIN_FREE_GB    stop once the filesystem has less free than this [120]
//   DCB_DRAIN_SECS     drain duration cap                               [300]
//   DCB_CONSUMER_RATE  live mode: payloads/s the consumer takes, 0 = max [0]
//   DCB_DEVICE         /proc/diskstats device to sample                 [nvme1n1]
//   DCB_TS_OUT         per-second CSV time series                       [none]
//
// Linux only: it samples /proc/diskstats and /proc/self/stat and evicts the
// page cache with posix_fadvise, none of which macOS has.
#![cfg(target_os = "linux")]

use std::borrow::Cow;
use std::ffi::CString;
use std::fmt::Write as _;
use std::io::Write as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use smol_str::SmolStr;

use patrolli::chan::{init_database, new as new_channel};
use patrolli::common::internal_metrics::Channel as ChannelMetrics;
use patrolli::config;
use patrolli::config::global::DiskChannel as GlobalDiskChannel;
use patrolli::payload::{LogRow, LogsBuilder, Payload, Scope};

const TOPIC: &str = "bench";
const TEMPLATES: usize = 16;
const GB: f64 = 1024.0 * 1024.0 * 1024.0;
const MB: f64 = 1024.0 * 1024.0;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Topic(&'static str);

impl std::fmt::Display for Topic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl AsRef<[u8]> for Topic {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

fn env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

struct Conf {
    dir: PathBuf,
    mode: String,
    rows: usize,
    comp: config::OtapCompression,
    comp_name: String,
    transport: bool,
    workers: usize,
    buffer: usize,
    producers: usize,
    consumers: usize,
    secs: u64,
    max_bytes: u64,
    min_free_bytes: u64,
    drain_secs: u64,
    consumer_rate: u64,
    device: String,
    ts_out: Option<PathBuf>,
}

impl Conf {
    fn from_env() -> Self {
        let comp_name: String = env("DCB_COMP", "zstd".to_string());
        let comp = match comp_name.as_str() {
            "none" => config::OtapCompression::None,
            "lz4" => config::OtapCompression::Lz4,
            "zstd" => config::OtapCompression::Zstd,
            other => panic!("unknown DCB_COMP {other}"),
        };
        let transport = env("DCB_TRANSPORT", 0u8) == 1;
        // Reported with the codec, so results of the two layouts stay apart.
        let comp_name = if transport {
            format!("{comp_name}+transport")
        } else {
            comp_name
        };
        Self {
            dir: PathBuf::from(std::env::var("DCB_DIR").expect("DCB_DIR is required")),
            mode: env("DCB_MODE", "spill_drain".to_string()),
            rows: env("DCB_ROWS", 1000),
            comp,
            comp_name,
            transport,
            workers: env("DCB_WORKERS", 4),
            buffer: env("DCB_BUFFER", 10),
            producers: env("DCB_PRODUCERS", 4),
            consumers: env("DCB_CONSUMERS", 4),
            secs: env("DCB_SECS", 60),
            max_bytes: (env("DCB_MAX_GB", 30.0f64) * GB) as u64,
            min_free_bytes: (env("DCB_MIN_FREE_GB", 120.0f64) * GB) as u64,
            drain_secs: env("DCB_DRAIN_SECS", 300),
            consumer_rate: env("DCB_CONSUMER_RATE", 0),
            device: env("DCB_DEVICE", "nvme1n1".to_string()),
            ts_out: std::env::var("DCB_TS_OUT").ok().map(PathBuf::from),
        }
    }
}

// ---------------------------------------------------------------- payloads

/// A syslog-shaped log payload. The bodies carry random host names, pids,
/// addresses and key fingerprints so compression sees something closer to
/// real traffic than a repeated constant.
fn make_payload(rows: usize) -> Payload {
    const PROGS: &[&str] = &["sshd", "kernel", "systemd", "nginx", "cron", "dhclient"];
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut builder = LogsBuilder::new();
    builder.scope(Scope {
        name: SmolStr::new_static("bench_disk_channel"),
        listener_id: None,
        listener_type: Some(SmolStr::new_static("syslog")),
        listener_name: Some(SmolStr::new_static("bench_disk_channel")),
        monitor_type: None,
        node_id: None,
    });
    let mut body = String::with_capacity(256);
    for _ in 0..rows {
        body.clear();
        let fp: String = (0..22)
            .map(|_| B64[rand::random_range(0..B64.len())] as char)
            .collect();
        let _ = write!(
            body,
            "<134>Oct  5 {:02}:{:02}:{:02} host-{:04} {}[{}]: Accepted publickey for user{} from 10.{}.{}.{} port {} ssh2: RSA SHA256:{}",
            rand::random_range(0..24u32),
            rand::random_range(0..60u32),
            rand::random_range(0..60u32),
            rand::random_range(0..2000u32),
            PROGS[rand::random_range(0..PROGS.len())],
            rand::random_range(100..65535u32),
            rand::random_range(0..500u32),
            rand::random_range(0..256u32),
            rand::random_range(0..256u32),
            rand::random_range(0..256u32),
            rand::random_range(1024..65535u32),
            fp,
        );
        builder.push_row(LogRow {
            body: Some(Cow::Borrowed(&body)),
            observed_time: None,
            severity: None,
            source_port: Some(rand::random_range(1024..65535u16)),
            source_ip: Some(IpAddr::from(rand::random::<[u8; 4]>())),
            attrs: &[],
        });
    }
    builder.finish()
}

/// Bytes per payload as the channel encodes it: uncompressed, and as stored.
fn encoded_sizes(pool: &[Payload], comp: config::OtapCompression, transport: bool) -> (f64, f64) {
    let (mut raw, mut stored) = (0usize, 0usize);
    for p in pool {
        raw += patrolli::chan::codec::encode(p, config::OtapCompression::None, transport)
            .unwrap()
            .len();
        stored += patrolli::chan::codec::encode(p, comp, transport)
            .unwrap()
            .len();
    }
    (
        raw as f64 / pool.len() as f64,
        stored as f64 / pool.len() as f64,
    )
}

// ---------------------------------------------------------------- system counters

#[derive(Clone, Copy, Default)]
struct DiskStat {
    reads: u64,
    read_sectors: u64,
    writes: u64,
    write_sectors: u64,
    io_ms: u64,
}

fn disk_stat(device: &str) -> DiskStat {
    let text = std::fs::read_to_string("/proc/diskstats").unwrap();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() > 13 && f[2] == device {
            let n = |i: usize| f[i].parse::<u64>().unwrap();
            return DiskStat {
                reads: n(3),
                read_sectors: n(5),
                writes: n(7),
                write_sectors: n(9),
                io_ms: n(12),
            };
        }
    }
    panic!("device {device} not in /proc/diskstats");
}

/// CPU seconds this process has used (user + system).
fn cpu_secs() -> f64 {
    let text = std::fs::read_to_string("/proc/self/stat").unwrap();
    // Fields after the parenthesised comm, which may itself contain spaces.
    let rest = &text[text.rfind(')').unwrap() + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks = f[11].parse::<u64>().unwrap() + f[12].parse::<u64>().unwrap();
    ticks as f64 / unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64
}

fn free_bytes(path: &Path) -> u64 {
    let c = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::statvfs(c.as_ptr(), &mut s) }, 0);
    s.f_bavail as u64 * s.f_frsize as u64
}

fn dir_bytes(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                total += dir_bytes(&e.path());
            } else {
                total += md.len();
            }
        }
    }
    total
}

/// Flushes every file under `path` and drops it from the page cache, so the
/// drain reads from the device rather than from memory it was written to.
fn make_cold(path: &Path) {
    let Ok(rd) = std::fs::read_dir(path) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            make_cold(&p);
        } else if let Ok(f) = std::fs::File::open(&p) {
            use std::os::fd::AsRawFd;
            let _ = f.sync_all();
            unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
        }
    }
}

// ---------------------------------------------------------------- sampling

struct Counters {
    sent: AtomicU64,
    received: AtomicU64,
}

/// One row of the per-second series, and the deltas behind each phase total.
struct Sampler {
    device: String,
    dir: PathBuf,
    metrics: Arc<ArcSwap<(Instant, ChannelMetrics)>>,
    ts: Option<std::fs::File>,
    t0: Instant,
    last_t: Instant,
    last_disk: DiskStat,
    last_cpu: f64,
    last_sent: u64,
    last_received: u64,
}

struct Sample {
    disk_bytes: u64,
    free: u64,
}

impl Sampler {
    fn new(conf: &Conf, metrics: Arc<ArcSwap<(Instant, ChannelMetrics)>>) -> Self {
        let mut ts = conf
            .ts_out
            .as_ref()
            .map(|p| std::fs::File::create(p).unwrap());
        if let Some(f) = ts.as_mut() {
            writeln!(
                f,
                "t,phase,sent_per_s,recv_per_s,wr_mb_s,wr_iops,rd_mb_s,rd_iops,util_pct,cpu_cores,topic_disk_gb,free_gb"
            )
            .unwrap();
        }
        let now = Instant::now();
        Self {
            device: conf.device.clone(),
            dir: conf.dir.clone(),
            metrics,
            ts,
            t0: now,
            last_t: now,
            last_disk: disk_stat(&conf.device),
            last_cpu: cpu_secs(),
            last_sent: 0,
            last_received: 0,
        }
    }

    fn tick(&mut self, phase: &str, c: &Counters) -> Sample {
        let now = Instant::now();
        let dt = now.duration_since(self.last_t).as_secs_f64();
        let d = disk_stat(&self.device);
        let cpu = cpu_secs();
        let sent = c.sent.load(Ordering::Relaxed);
        let received = c.received.load(Ordering::Relaxed);
        let disk_bytes = self
            .metrics
            .load()
            .1
            .channels
            .get(TOPIC)
            .map(|s| s.length)
            .unwrap_or_default();
        let free = free_bytes(&self.dir);
        if let Some(f) = self.ts.as_mut() {
            writeln!(
                f,
                "{:.1},{},{:.1},{:.1},{:.1},{:.0},{:.1},{:.0},{:.1},{:.2},{:.2},{:.1}",
                now.duration_since(self.t0).as_secs_f64(),
                phase,
                (sent - self.last_sent) as f64 / dt,
                (received - self.last_received) as f64 / dt,
                (d.write_sectors - self.last_disk.write_sectors) as f64 * 512.0 / MB / dt,
                (d.writes - self.last_disk.writes) as f64 / dt,
                (d.read_sectors - self.last_disk.read_sectors) as f64 * 512.0 / MB / dt,
                (d.reads - self.last_disk.reads) as f64 / dt,
                (d.io_ms - self.last_disk.io_ms) as f64 / (dt * 10.0),
                (cpu - self.last_cpu) / dt,
                disk_bytes as f64 / GB,
                free as f64 / GB,
            )
            .unwrap();
        }
        self.last_t = now;
        self.last_disk = d;
        self.last_cpu = cpu;
        self.last_sent = sent;
        self.last_received = received;
        Sample { disk_bytes, free }
    }
}

/// Totals for one phase, from counters taken at its start and end.
struct Phase {
    t: Instant,
    disk: DiskStat,
    cpu: f64,
    sent: u64,
    received: u64,
}

impl Phase {
    fn start(device: &str, c: &Counters) -> Self {
        Self {
            t: Instant::now(),
            disk: disk_stat(device),
            cpu: cpu_secs(),
            sent: c.sent.load(Ordering::Relaxed),
            received: c.received.load(Ordering::Relaxed),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn report(&self, name: &str, conf: &Conf, c: &Counters, raw: f64, stored: f64, extra: &str) {
        let secs = self.t.elapsed().as_secs_f64();
        let d = disk_stat(&conf.device);
        let payloads = match name {
            "drain" => c.received.load(Ordering::Relaxed) - self.received,
            _ => c.sent.load(Ordering::Relaxed) - self.sent,
        };
        let received = c.received.load(Ordering::Relaxed) - self.received;
        let wr_bytes = (d.write_sectors - self.disk.write_sectors) as f64 * 512.0;
        let rd_bytes = (d.read_sectors - self.disk.read_sectors) as f64 * 512.0;
        let writes = (d.writes - self.disk.writes) as f64;
        let reads = (d.reads - self.disk.reads) as f64;
        let logical = payloads as f64 * stored;
        println!(
            "DCB_RESULT {{\"phase\":\"{name}\",\"mode\":\"{}\",\"rows\":{},\"comp\":\"{}\",\"workers\":{},\"buffer\":{},\"producers\":{},\"consumers\":{},\
\"secs\":{secs:.2},\"payloads\":{payloads},\"received\":{received},\"payloads_s\":{:.1},\"rows_s\":{:.0},\
\"raw_mb_s\":{:.1},\"stored_mb_s\":{:.1},\"payload_raw_kb\":{:.1},\"payload_stored_kb\":{:.1},\
\"dev_wr_mb_s\":{:.1},\"dev_wr_iops\":{:.0},\"dev_wr_avg_kb\":{:.1},\"dev_rd_mb_s\":{:.1},\"dev_rd_iops\":{:.0},\"dev_rd_avg_kb\":{:.1},\
\"dev_util_pct\":{:.1},\"cpu_cores\":{:.2},\"write_amp\":{:.2}{extra}}}",
            conf.mode,
            conf.rows,
            conf.comp_name,
            conf.workers,
            conf.buffer,
            conf.producers,
            conf.consumers,
            payloads as f64 / secs,
            payloads as f64 * conf.rows as f64 / secs,
            payloads as f64 * raw / MB / secs,
            logical / MB / secs,
            raw / 1024.0,
            stored / 1024.0,
            wr_bytes / MB / secs,
            writes / secs,
            if writes > 0.0 {
                wr_bytes / writes / 1024.0
            } else {
                0.0
            },
            rd_bytes / MB / secs,
            reads / secs,
            if reads > 0.0 {
                rd_bytes / reads / 1024.0
            } else {
                0.0
            },
            (d.io_ms - self.disk.io_ms) as f64 / (secs * 10.0),
            (cpu_secs() - self.cpu) / secs,
            if logical > 0.0 {
                wr_bytes / logical
            } else {
                0.0
            },
        );
    }
}

// ---------------------------------------------------------------- the run

fn spawn_producers(
    conf: &Conf,
    send: &patrolli::chan::SenderChannel<Topic, Payload>,
    pool: &Arc<Vec<Payload>>,
    counters: &Arc<Counters>,
    stop: &Arc<AtomicBool>,
) -> Vec<std::thread::JoinHandle<()>> {
    (0..conf.producers)
        .map(|i| {
            let send = send.clone();
            let pool = pool.clone();
            let counters = counters.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let topic = Topic(TOPIC);
                let mut n = i;
                while !stop.load(Ordering::Relaxed) {
                    let p = pool[n % pool.len()].clone();
                    n += 1;
                    if send.send_to(&topic, p).is_err() {
                        break;
                    }
                    counters.sent.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "writes tens of GB to DCB_DIR; run explicitly with --ignored"]
async fn disk_channel_throughput() {
    let conf = Conf::from_env();
    std::fs::create_dir_all(&conf.dir).unwrap();
    assert!(
        free_bytes(&conf.dir) > conf.min_free_bytes,
        "not enough free space to start"
    );
    init_database(&GlobalDiskChannel {
        data_dir: conf.dir.to_string_lossy().into_owned(),
        ..Default::default()
    })
    .unwrap();

    let pool: Arc<Vec<Payload>> =
        Arc::new((0..TEMPLATES).map(|_| make_payload(conf.rows)).collect());
    let (raw, stored) = encoded_sizes(&pool, conf.comp, conf.transport);
    eprintln!(
        "payload: {} rows, {:.1} KiB encoded, {:.1} KiB stored ({})",
        conf.rows,
        raw / 1024.0,
        stored / 1024.0,
        conf.comp_name
    );

    let metrics = Arc::new(ArcSwap::from_pointee((
        Instant::now(),
        ChannelMetrics::default(),
    )));
    let (admin, send) = new_channel::<Topic, Payload>(Duration::from_millis(500), metrics.clone());
    let topic = Topic(TOPIC);
    let receiver = admin
        .get_receiver(
            &topic,
            config::Channel::Disk(config::DiskChannel {
                non_blocking: false,
                max_memtable_size: 64 * 1024 * 1024,
                // The harness bounds the size itself: a full topic would park
                // the producers in its backpressure loop where they cannot
                // see the stop flag.
                max_disk_size: 0,
                worker_count: conf.workers,
                compression: conf.comp,
                transport_optimized: conf.transport,
                channel_buffer_size: conf.buffer,
                write_through: false,
            }),
        )
        .await
        .unwrap();

    let counters = Arc::new(Counters {
        sent: AtomicU64::new(0),
        received: AtomicU64::new(0),
    });
    let mut sampler = Sampler::new(&conf, metrics.clone());
    let stop = Arc::new(AtomicBool::new(false));

    // Consumers: count what arrives, optionally at a fixed overall rate.
    let consuming = Arc::new(AtomicBool::new(conf.mode == "live"));
    let consumers: Vec<_> = (0..conf.consumers)
        .map(|_| {
            let rx = receiver.clone();
            let counters = counters.clone();
            let consuming = consuming.clone();
            let per_task = conf.consumer_rate as f64 / conf.consumers as f64;
            tokio::spawn(async move {
                while !consuming.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                let start = Instant::now();
                let mut taken = 0u64;
                while let Ok(p) = rx.recv_async().await {
                    drop(p);
                    taken += 1;
                    counters.received.fetch_add(1, Ordering::Relaxed);
                    if per_task > 0.0 {
                        let due = Duration::from_secs_f64(taken as f64 / per_task);
                        if let Some(wait) = due.checked_sub(start.elapsed()) {
                            tokio::time::sleep(wait).await;
                        }
                    }
                }
            })
        })
        .collect();

    let phase_name = if conf.mode == "live" { "live" } else { "spill" };
    let phase = Phase::start(&conf.device, &counters);
    let producers = spawn_producers(&conf, &send, &pool, &counters, &stop);
    let deadline = Instant::now() + Duration::from_secs(conf.secs);
    let mut stop_reason = "time";
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.tick().await;
    loop {
        tick.tick().await;
        let s = sampler.tick(phase_name, &counters);
        if s.disk_bytes >= conf.max_bytes {
            stop_reason = "max_bytes";
        } else if s.free <= conf.min_free_bytes {
            stop_reason = "min_free";
        } else if Instant::now() < deadline {
            continue;
        }
        break;
    }
    stop.store(true, Ordering::Relaxed);
    for p in producers {
        p.join().unwrap();
    }
    phase.report(
        phase_name,
        &conf,
        &counters,
        raw,
        stored,
        &format!(
            ",\"stop\":\"{stop_reason}\",\"topic_disk_gb\":{:.2},\"dir_gb\":{:.2}",
            sampler.tick(phase_name, &counters).disk_bytes as f64 / GB,
            dir_bytes(&conf.dir) as f64 / GB
        ),
    );

    if conf.mode != "live" {
        // Let the bridge finish writing what the producers handed it.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let t = Instant::now();
        make_cold(&conf.dir);
        eprintln!(
            "backlog flushed and evicted in {:.1}s",
            t.elapsed().as_secs_f64()
        );
        sampler.tick("cold", &counters);

        let total = counters.sent.load(Ordering::Relaxed);
        let phase = Phase::start(&conf.device, &counters);
        consuming.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(conf.drain_secs);
        let mut stop_reason = "empty";
        loop {
            tick.tick().await;
            sampler.tick("drain", &counters);
            if counters.received.load(Ordering::Relaxed) >= total {
                break;
            }
            if Instant::now() >= deadline {
                stop_reason = "time";
                break;
            }
        }
        phase.report(
            "drain",
            &conf,
            &counters,
            raw,
            stored,
            &format!(",\"stop\":\"{stop_reason}\",\"backlog\":{total}"),
        );
    }

    // Removing the topic marks its keyspace for deletion; dropping every
    // receiver lets the bridge exit and delete it.
    drop(send);
    admin.remove(&topic).await.unwrap();
    drop(receiver);
    for c in consumers {
        c.abort();
    }
    std::io::stdout().flush().unwrap();
}
