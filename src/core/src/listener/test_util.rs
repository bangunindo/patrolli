// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The harness of the transport listeners' close and restart tests: one
//! listener run by a [`Worker`], as `WorkerGroup` runs it, feeding a processor
//! topic that holds one payload, so a test that does not read it backs the
//! pipeline up as far as the transport.

use super::{Listener, Worker};
use crate::chan::{AdminChannel, SenderChannel};
use crate::common::internal_metrics;
use crate::common::shutdown::Deadline;
use crate::processor::PROCESSOR_CHANNEL_NAME;
use crate::{cache, config, payload};
use arc_swap::ArcSwap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Generous next to the drain itself, which takes milliseconds.
pub(super) const CLOSE_LIMIT: Duration = Duration::from_secs(10);
/// Units the harness runs.
pub(super) const WORKERS: usize = 2;

pub(super) type ProcessorSender = SenderChannel<&'static str, payload::ProcessorPayload>;

/// What the harness reads of a listener's transport.
pub(super) trait Transport: Listener + 'static {
    /// Started, with its intake on.
    async fn running(&self) -> bool;
}

/// The kind of port the listener binds, which decides how it is probed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Port {
    Udp,
    Tcp,
}

/// A cache where every record of `listener_id` belongs to one item, so
/// annotation sends it on instead of dropping it.
pub(super) fn item_for(listener_id: Uuid) -> cache::Cache {
    let cache = cache::Cache::new();
    let host_id = Uuid::now_v7();
    cache.host.write().insert(config::host::Host {
        id: host_id,
        name: "host".into(),
        timezone: None,
    });
    let item = config::item::Item::new(
        Uuid::now_v7(),
        "listener-item".into(),
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

/// The processor topic a listener sends into, holding `buffer` payloads, and
/// its reading end.
pub(super) async fn processor_topic(
    buffer: usize,
) -> (
    AdminChannel<&'static str, payload::ProcessorPayload>,
    ProcessorSender,
    flume::Receiver<payload::ProcessorPayload>,
) {
    let (admin, sender) = crate::chan::new(
        Duration::from_secs(3600),
        Arc::new(ArcSwap::from_pointee((
            std::time::Instant::now(),
            internal_metrics::Channel::default(),
        ))),
    );
    let rx = admin
        .get_receiver(
            &PROCESSOR_CHANNEL_NAME,
            config::Channel::Memory(config::MemoryChannel {
                non_blocking: false,
                channel_buffer_size: buffer,
            }),
        )
        .await
        .unwrap();
    (admin, sender, rx)
}

pub(super) struct Harness<L> {
    pub listener: Arc<L>,
    pub worker: Worker,
    processor_rx: flume::Receiver<payload::ProcessorPayload>,
    _admin: AdminChannel<&'static str, payload::ProcessorPayload>,
    pub addr: SocketAddr,
}

impl<L: Transport> Harness<L> {
    /// Starts [`WORKERS`] units of the listener `build` makes for a fresh
    /// port, retrying when another test took the port between the probe and
    /// the bind.
    pub async fn start(
        port: Port,
        drain_timeout: Duration,
        build: impl Fn(SocketAddr, ProcessorSender) -> (config::listener::Listener, L),
    ) -> Self {
        for _ in 0..5 {
            if let Some(h) = Self::try_start(port, drain_timeout, &build).await {
                return h;
            }
        }
        panic!("listener did not start on any probed port");
    }

    async fn try_start(
        port: Port,
        drain_timeout: Duration,
        build: &impl Fn(SocketAddr, ProcessorSender) -> (config::listener::Listener, L),
    ) -> Option<Self> {
        let (admin, sender, processor_rx) = processor_topic(1).await;
        // UDP: the probe keeps the port until the listener's reuseport socket
        // has joined it, so no port-0 bind elsewhere can take it in between.
        // TCP cannot share a port that way, so a lost race shows up as a
        // failed start and is retried.
        let probe = (port == Port::Udp).then(|| {
            let probe =
                socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
            probe.set_reuse_port(true).unwrap();
            probe
                .bind(&"127.0.0.1:0".parse::<SocketAddr>().unwrap().into())
                .unwrap();
            probe
        });
        let addr = match &probe {
            Some(probe) => probe.local_addr().unwrap().as_socket().unwrap(),
            None => std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap(),
        };
        let (conf, listener) = build(addr, sender);
        let listener = Arc::new(listener);
        let mut worker = Worker::with_listener(conf, listener.clone());
        worker.drain_timeout = drain_timeout;
        worker.adjust(WORKERS).await;
        let h = Self {
            listener,
            worker,
            processor_rx,
            _admin: admin,
            addr,
        };
        let deadline = Instant::now() + CLOSE_LIMIT;
        while h.workers().await != WORKERS || !h.listener.running().await {
            // A unit returns on its own only when the transport failed to
            // start, i.e. the bind failed.
            if h.any_unit_finished().await {
                h.worker.close(Deadline::NONE).await;
                return None;
            }
            assert!(Instant::now() < deadline, "listener did not start");
            sleep(Duration::from_millis(10)).await;
        }
        // Sharing the port, it would take a share of the datagrams.
        drop(probe);
        Some(h)
    }

    pub async fn workers(&self) -> usize {
        self.listener.metrics().await.total_workers
    }

    pub async fn running(&self) -> bool {
        self.listener.running().await
    }

    async fn any_unit_finished(&self) -> bool {
        let units = self.worker.units.lock().await;
        units.live.iter().any(|unit| unit.handle.is_finished())
    }

    /// Bytes the transport has taken in.
    pub async fn intake_bytes(&self) -> u64 {
        self.listener.metrics().await.total_bytes
    }

    /// Waits until intake reaches `bytes`.
    pub async fn wait_for_intake(&self, bytes: u64) {
        let deadline = Instant::now() + CLOSE_LIMIT;
        loop {
            let now = self.intake_bytes().await;
            if now >= bytes {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "intake took {now} of {bytes} bytes"
            );
            sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits until intake stops moving, i.e. the pipeline is backed up as far
    /// as the transport.
    pub async fn wait_for_stalled_intake(&self) -> u64 {
        let deadline = Instant::now() + CLOSE_LIMIT;
        let mut last = u64::MAX;
        loop {
            sleep(Duration::from_millis(200)).await;
            let now = self.intake_bytes().await;
            if now == last && now > 0 {
                return now;
            }
            assert!(
                Instant::now() < deadline,
                "intake did not stall with data: {now} bytes"
            );
            last = now;
        }
    }

    /// Waits until every unit has returned on its own.
    pub async fn wait_for_no_workers(&self) {
        let deadline = Instant::now() + CLOSE_LIMIT;
        while self.workers().await != 0 {
            assert!(Instant::now() < deadline, "workers did not return");
            sleep(Duration::from_millis(10)).await;
        }
    }

    /// Restarts the units as the supervisor does once its backoff has
    /// passed, and waits for the transport to run again.
    pub async fn restart_workers(&self) {
        self.worker.adjust(WORKERS).await;
        let deadline = Instant::now() + CLOSE_LIMIT;
        while self.workers().await != WORKERS || !self.running().await {
            assert!(Instant::now() < deadline, "transport did not restart");
            sleep(Duration::from_millis(10)).await;
        }
    }

    /// Starts reading the pipeline; the counter is the rows read.
    pub fn consume(&self) -> (Arc<AtomicUsize>, JoinHandle<()>) {
        let rx = self.processor_rx.clone();
        let rows = Arc::new(AtomicUsize::new(0));
        let counted = rows.clone();
        let consumer = tokio::spawn(async move {
            while let Ok(pl) = rx.recv_async().await {
                counted.fetch_add(pl.payload.num_rows(), Ordering::Relaxed);
            }
        });
        (rows, consumer)
    }

    /// Waits until the pipeline has seen `want` rows.
    pub async fn wait_for_rows(rows: &AtomicUsize, want: usize) {
        let deadline = Instant::now() + CLOSE_LIMIT;
        while rows.load(Ordering::Relaxed) < want {
            assert!(
                Instant::now() < deadline,
                "pipeline saw {} of {want} rows",
                rows.load(Ordering::Relaxed)
            );
            sleep(Duration::from_millis(10)).await;
        }
    }

    /// Closes the listener as `WorkerGroup` does. With `consume`, the
    /// pipeline starts reading at the same moment; returns the records it
    /// received.
    pub async fn close(self, consume: bool) -> (Duration, Option<usize>) {
        let started = Instant::now();
        let consumer = consume.then(|| self.consume());
        let closed = timeout(CLOSE_LIMIT, self.worker.close(Deadline::NONE)).await;
        assert!(
            closed.is_ok(),
            "close did not return within {CLOSE_LIMIT:?}"
        );
        let elapsed = started.elapsed();
        let rows = match consumer {
            Some((rows, consumer)) => {
                // Every unit is gone, so nothing more is coming. Wait for the
                // consumer to stop: one that has just taken a payload counts
                // it before its next await.
                consumer.abort();
                let _ = consumer.await;
                let buffered: usize = self
                    .processor_rx
                    .try_iter()
                    .map(|pl| pl.payload.num_rows())
                    .sum();
                Some(rows.load(Ordering::Relaxed) + buffered)
            }
            None => None,
        };
        (elapsed, rows)
    }
}
