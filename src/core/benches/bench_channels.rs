// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// Throughput benchmark for the three channel implementations:
//   1. old in-memory channel (`chan::memory::MemoryChannel`)
//   2. new in-memory channel (`chan::ChannelGroup` with `config::Channel::Memory`)
//   3. new persistent channel (`chan::ChannelGroup` with `config::Channel::Disk`)
//
// Each variant is exercised with a draining receiver running on a tokio worker
// pool, so what we measure is the cost of *one batch send* on the producer
// thread. Criterion's throughput reporting is set to `Elements(batch_size)` so
// the report shows both batches/sec (via mean iter time) and records/sec
// (via the throughput line).
//
// Usage:
//   cargo bench --bench bench_channels
//   cargo bench --bench bench_channels -- old_memory/1000

use std::borrow::Cow;
use std::net::IpAddr;
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use flume::Receiver;
use smol_str::SmolStr;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use patrolli::chan::{init_database, new as new_channel};
use patrolli::common::internal_metrics::Channel as ChannelMetrics;
use patrolli::config;
use patrolli::config::global::DiskChannel as GlobalDiskChannel;
use patrolli::payload::{LogRow, LogsBuilder, Payload, Scope};

// Topics need `AsRef<[u8]> + Display + Ord + Hash + ...` for the new
// `ChannelGroup`, so we use a tiny newtype around a `'static str`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct BenchTopic(&'static str);

impl std::fmt::Display for BenchTopic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl AsRef<[u8]> for BenchTopic {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

// Number of records per batch we sweep over.
const BATCH_SIZES: &[usize] = &[1, 10, 100, 1000];

// Backing channel buffer (in *batches*) for memory channels. Big enough that
// a single iteration almost never blocks on full buffer, but small enough that
// we still measure realistic pressure on the consumer.
const MEMORY_BUFFER_BATCHES: usize = 1024;

fn metrics_swap() -> Arc<ArcSwap<(Instant, ChannelMetrics)>> {
    Arc::new(ArcSwap::from_pointee((
        Instant::now(),
        ChannelMetrics::default(),
    )))
}

// Build a representative log `Payload` carrying `size` rows. Mirrors the shape
// the dummy listener produces (one scope + N rows with a body and source
// ip/port) so the disk channel's serialization cost is realistic.
fn make_batch(size: usize) -> Payload {
    let mut builder = LogsBuilder::new();
    builder.scope(Scope {
        name: SmolStr::new_static("bench_channels"),
        listener_id: None,
        listener_type: Some(SmolStr::new_static("dummy")),
        listener_name: Some(SmolStr::new_static("bench_channels")),
        monitor_type: None,
        node_id: None,
    });
    for _ in 0..size {
        builder.push_row(LogRow {
            body: Some(Cow::Borrowed("dummy message")),
            observed_time: None,
            severity: None,
            source_port: Some(0),
            source_ip: Some(IpAddr::from([127, 0, 0, 1])),
            attrs: &[],
        });
    }
    builder.finish()
}

// One-time global init of the fjall database used by the disk channel.
// Uses a unique temp dir per process invocation so repeated `cargo bench`
// runs don't share state.
fn ensure_disk_db() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!(
            "patrolli-bench-channel-{}",
            uuid::Uuid::now_v7().simple()
        ));
        std::fs::create_dir_all(&dir).expect("create bench db dir");
        let conf = GlobalDiskChannel {
            data_dir: dir.to_string_lossy().into_owned(),
            max_journal_size: 0,
            cache_size: 0,
        };
        init_database(&conf).expect("init bench db");
    });
}

// Each bench keeps its own dedicated topic; we leak the topic name into a
// `'static str` because the type signature demands it and the number of leaks
// is bounded (one per (impl, batch_size) pair).
fn topic_for_bench(prefix: &'static str, batch_size: usize) -> BenchTopic {
    let leaked: &'static str = Box::leak(format!("{prefix}-{batch_size}").into_boxed_str());
    BenchTopic(leaked)
}

fn spawn_drain(runtime: &Runtime, receiver: Receiver<Payload>) -> JoinHandle<()> {
    runtime.spawn(async move {
        while let Ok(_batch) = receiver.recv_async().await {
            // drop the batch immediately
        }
    })
}

fn bench_new_memory(c: &mut Criterion) {
    let runtime = Runtime::new().expect("tokio runtime");
    // `new_channel` spawns its metrics reporter, so it needs a runtime context.
    let _guard = runtime.enter();
    let mut group = c.benchmark_group("new_memory");

    for &batch_size in BATCH_SIZES {
        group.throughput(Throughput::Elements(batch_size as u64));

        let (chan_admin, chan_send) = new_channel(Duration::from_secs(60), metrics_swap());
        let conf = config::Channel::Memory(config::MemoryChannel {
            non_blocking: false,
            channel_buffer_size: MEMORY_BUFFER_BATCHES,
        });
        let topic = topic_for_bench("new_memory", batch_size);
        let receiver = runtime
            .block_on(chan_admin.get_receiver(&topic, conf))
            .unwrap();
        let drain = spawn_drain(&runtime, receiver);

        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &size| {
                let template = make_batch(size);
                b.iter_batched(
                    || template.clone(),
                    |batch| {
                        chan_send.send_to(&topic, batch).unwrap();
                    },
                    BatchSize::SmallInput,
                );
            },
        );

        drop(chan_send);
        runtime
            .block_on(async move {
                chan_admin.remove(&topic).await?;
                drain.await?;
                Ok::<(), anyhow::Error>(())
            })
            .unwrap();
    }

    group.finish();
}

fn bench_new_disk(c: &mut Criterion) {
    ensure_disk_db();
    let runtime = Runtime::new().expect("tokio runtime");
    // `new_channel` spawns its metrics reporter, so it needs a runtime context.
    let _guard = runtime.enter();
    let mut group = c.benchmark_group("new_disk");
    // Disk path has compression + fjall write overhead, so per-iter is much
    // slower than memory. Trim samples & extend measurement time to keep the
    // total bench duration reasonable.
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(15));

    for &batch_size in BATCH_SIZES {
        group.throughput(Throughput::Elements(batch_size as u64));

        let (chan_admin, chan_send) = new_channel(Duration::from_secs(60), metrics_swap());
        let conf = config::Channel::Disk(config::DiskChannel {
            non_blocking: false,
            max_memtable_size: 64 * 1024 * 1024,
            max_disk_size: 0, // unlimited
            worker_count: 4,
            compression: config::OtapCompression::Zstd,
            transport_optimized: false,
            channel_buffer_size: 0,
            write_through: false,
        });
        let topic = topic_for_bench("new_disk", batch_size);
        let receiver = runtime
            .block_on(chan_admin.get_receiver(&topic, conf))
            .unwrap();
        let drain = spawn_drain(&runtime, receiver);

        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &size| {
                let template = make_batch(size);
                b.iter_batched(
                    || template.clone(),
                    |batch| {
                        chan_send.send_to(&topic, batch).unwrap();
                    },
                    BatchSize::SmallInput,
                );
            },
        );

        drop(chan_send);
        runtime
            .block_on(async move {
                chan_admin.remove(&topic).await?;
                drain.await?;
                Ok::<(), anyhow::Error>(())
            })
            .unwrap();
    }

    group.finish();
}

criterion_group!(benches, bench_new_memory, bench_new_disk);
criterion_main!(benches);
