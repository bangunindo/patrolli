// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The `bgp` listener: a BMP server whose sessions write the route table, and
//! workers that hand the sessions' events to the pipeline.
//!
//! The transport belongs to the listener, as for syslog: the first worker
//! starts it and it outlives worker restarts. The sessions write the table
//! themselves, in stream order, so the workers only carry events; with none
//! wanted they idle. The collector (routers, `keep` deadlines) lives as long
//! as the transport; a new one re-arms `keep` for the routes the old one left
//! in the table.

use super::events::{Event, EventSink};
use super::session::{BmpConfig, Collector};
use super::stats::BmpStats;
use crate::chan::SenderChannel;
use crate::config::listener::{Bgp, ListenerKind};
use crate::listener::InternalMetrics;
use crate::payload::Scope;
use crate::{cache, common, config, payload};
use anyhow::{Context, Result};
use async_trait::async_trait;
use flume::Receiver;
use smol_str::SmolStr;
use socket2::{SockRef, TcpKeepalive};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::Mutex as TokioMutex;
use tokio::task::JoinSet;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, Span, debug, error, info, warn};
use uuid::Uuid;

/// What the transport is doing. Workers start it; only `stop` ends it.
enum State {
    Idle,
    Running(Running),
    /// Stopped by the listener's close; never started again.
    Closed,
}

/// A started transport. Only its tasks hold the collector, and with it the
/// sender of `rx`'s channel, so the workers see the channel disconnect once
/// the server and the expiry task have returned.
struct Running {
    rx: Receiver<Event>,
    /// Stops the intake: the server, its sessions and the expiry task. A
    /// server that returns on its own cancels it too.
    intake: CancellationToken,
    tasks: JoinSet<()>,
}

impl Running {
    async fn shutdown(mut self) {
        self.intake.cancel();
        drop(self.rx);
        while let Some(res) = self.tasks.join_next().await {
            if let Err(e) = res {
                error!(error = %e, "transport task failed");
            }
        }
    }
}

pub struct BgpListener {
    conf: config::listener::Listener,
    bgp: Bgp,
    channel: SenderChannel<&'static str, payload::ProcessorPayload>,
    forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
    conf_cache: cache::Cache,
    node_id: Option<Uuid>,

    internal_metrics: InternalMetrics,
    /// kept across transport restarts, like the generic counters
    stats: Arc<BmpStats>,
    events_dropped: Arc<AtomicU64>,
    /// the running transport's collector, for the gauges
    collector: parking_lot::Mutex<Weak<Collector>>,
    state: TokioMutex<State>,
}

impl BgpListener {
    pub fn new(
        conf: config::listener::Listener,
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
    ) -> Self {
        let config::listener::Config::Bgp(bgp) = &conf.config else {
            unreachable!("a bgp listener is built from a bgp config");
        };
        Self {
            bgp: (**bgp).clone(),
            conf,
            channel,
            forwarder,
            conf_cache,
            node_id,
            internal_metrics: InternalMetrics::default(),
            stats: Arc::default(),
            events_dropped: Arc::default(),
            collector: parking_lot::Mutex::new(Weak::new()),
            state: TokioMutex::new(State::Idle),
        }
    }

    /// The collector of the running transport, if any.
    pub fn collector(&self) -> Option<Arc<Collector>> {
        self.collector.lock().upgrade()
    }

    /// Returns the channel the workers read, starting the transport unless it
    /// runs. `None` once the listener is closed.
    async fn ensure_started(&self) -> Result<Option<Receiver<Event>>> {
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
        let running = self.start().await?;
        let rx = running.rx.clone();
        *state = State::Running(running);
        Ok(Some(rx))
    }

    async fn start(&self) -> Result<Running> {
        let bmp = self.bgp.bmp.as_ref().context("no bmp section")?;
        let socket = TcpListener::bind(bmp.listener.address)
            .await
            .with_context(|| format!("bind bmp {}", bmp.listener.address))?;
        let (tx, rx) = flume::bounded(self.bgp.events.buffer_size);
        let vrfs = (!self.bgp.vrfs.is_empty()).then(|| self.bgp.vrfs.iter().cloned().collect());
        let collector = Collector::new(
            self.conf.id,
            self.conf_cache.lookup_table.clone(),
            BmpConfig {
                keep: bmp.keep,
                max_message_size: bmp.max_message_size,
                vrfs,
            },
        )
        .with_stats(self.stats.clone())
        .with_events(EventSink::new(
            tx,
            self.bgp.events.routes,
            self.bgp.events.on_full,
            self.events_dropped.clone(),
        ));
        let collector = Arc::new(collector);
        *self.collector.lock() = Arc::downgrade(&collector);

        let intake = CancellationToken::new();
        let mut tasks = JoinSet::new();
        let filter = IpFilter {
            whitelist: bmp.listener.ip_whitelist.clone(),
            blacklist: bmp.listener.ip_blacklist.clone(),
        };
        tasks.spawn(
            server(socket, filter, collector.clone(), intake.clone()).instrument(Span::current()),
        );
        let expiry_intake = intake.clone();
        tasks.spawn(
            async move {
                // the server going down takes the intake with it
                collector.run_expiry(expiry_intake).await;
            }
            .instrument(Span::current()),
        );
        info!(transport = "bmp", addr = %bmp.listener.address, "server listening");
        Ok(Running { rx, intake, tasks })
    }

    /// Stops the transport for good: the sessions end without arming `keep`
    /// (their routes stay until the table goes or a later transport expires
    /// them), and the channel disconnects once they have sent their events.
    async fn stop(&self) {
        let state = std::mem::replace(&mut *self.state.lock().await, State::Closed);
        if let State::Running(running) = state {
            running.shutdown().await;
            debug!("transport stopped");
        }
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

    fn push(&self, batch: &mut payload::LogsBuilder, event: &Event) {
        event.push(batch);
        self.internal_metrics
            .total_records
            .fetch_add(1, Ordering::Relaxed);
    }

    async fn send_batch(&self, batch: &mut payload::LogsBuilder) {
        if batch.is_empty() {
            return;
        }
        let pl = batch.finish();
        super::super::send_batch(super::super::SendBatch {
            batch: pl,
            conf: &self.conf,
            conf_cache: &self.conf_cache,
            node_id: self.node_id,
            internal_metrics: &self.internal_metrics,
            channel: &self.channel,
            forwarder: &self.forwarder,
        })
        .await;
        self.set_scope(batch);
    }
}

#[derive(Debug)]
struct IpFilter {
    whitelist: Vec<IpAddr>,
    blacklist: Vec<IpAddr>,
}

/// Accepts BMP connections until `intake` is cancelled, then ends the
/// sessions and waits for them.
async fn server(
    socket: TcpListener,
    filter: IpFilter,
    collector: Arc<Collector>,
    intake: CancellationToken,
) {
    // However this returns, the rest of the intake goes down with it.
    let _intake_guard = intake.clone().drop_guard();
    let mut sessions = JoinSet::new();
    loop {
        while sessions.try_join_next().is_some() {}
        let (stream, addr) = tokio::select! {
            biased;

            _ = intake.cancelled() => break,
            accepted = socket.accept() => match accepted {
                Ok(v) => v,
                Err(e) => {
                    warn!(transport = "bmp", error = %e, "accept failed");
                    continue;
                }
            },
        };
        if !common::ip_allowed(&addr.ip(), &filter.whitelist, &filter.blacklist) {
            debug!(peer = %addr, "connection rejected by ip filter");
            continue;
        }
        let ka = TcpKeepalive::new()
            .with_time(Duration::from_secs(60))
            .with_interval(Duration::from_secs(20))
            .with_retries(4);
        if let Err(e) = SockRef::from(&stream).set_tcp_keepalive(&ka) {
            debug!(peer = %addr, error = %e, "set tcp keepalive failed");
        }
        let (collector, cancel) = (collector.clone(), intake.child_token());
        sessions.spawn(
            async move {
                collector.session(stream, addr.ip(), cancel).await;
            }
            .instrument(Span::current()),
        );
    }
    // Free the port, then end the sessions for good: a stopping listener arms
    // no `keep`.
    drop(socket);
    collector.close_all();
    while let Some(res) = sessions.join_next().await {
        if let Err(e) = res {
            error!(transport = "bmp", error = %e, "session task failed");
        }
    }
    info!(transport = "bmp", "server stopped");
}

#[async_trait]
impl crate::listener::Listener for BgpListener {
    #[tracing::instrument(skip_all, fields(listener_id = %self.conf.id, listener = %self.conf.name))]
    async fn listen(&self, token: CancellationToken) {
        let recv = match self.ensure_started().await {
            Ok(Some(recv)) => recv,
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
        let batch_size = self.conf.batch.batch_size;
        let mut batch = payload::LogsBuilder::new();
        self.set_scope(&mut batch);
        let mut tick = interval(self.conf.batch.batch_timeout);
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
                    self.send_batch(&mut batch).await;
                }
                res = recv.recv_async() => {
                    self.internal_metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    // Disconnected: the transport stopped, or died and the
                    // supervisor restarts this worker.
                    let Ok(event) = res else {
                        debug!("transport closed");
                        break;
                    };
                    self.push(&mut batch, &event);
                    if batch.num_items() >= batch_size {
                        self.send_batch(&mut batch).await;
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
            for event in recv.drain() {
                self.push(&mut batch, &event);
                if batch.num_items() >= batch_size {
                    self.send_batch(&mut batch).await;
                }
            }
        }
        self.send_batch(&mut batch).await;
        info!("listener stopped");
        self.internal_metrics
            .total_workers
            .fetch_sub(1, Ordering::Relaxed);
    }

    async fn stop(&self) {
        BgpListener::stop(self).await;
    }

    async fn metrics(&self) -> common::internal_metrics::Listener {
        let mut m = self
            .internal_metrics
            .get(self.conf.id, self.conf.name.to_string());
        let dropped = self.events_dropped.load(Ordering::Relaxed);
        m.total_bytes = BmpStats::get(&self.stats.bytes);
        m.total_items = self.stats.total_messages();
        m.total_records_dropped += dropped;
        let mut bgp = self.stats.snapshot();
        bgp.routers_live = self.collector().map_or(0, |c| c.routers().live() as u64);
        bgp.route_table = self
            .conf_cache
            .lookup_table
            .route_table_for_listener(&self.conf.id)
            .is_some();
        bgp.events_dropped = dropped;
        m.bgp = Some(Box::new(bgp));
        m
    }
}

#[cfg(test)]
impl crate::listener::test_util::Transport for BgpListener {
    async fn running(&self) -> bool {
        matches!(
            &*self.state.lock().await,
            State::Running(running) if !running.intake.is_cancelled()
        )
    }
}
