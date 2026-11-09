// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The `bgp` listener: a BMP server and a passive BGP server whose sessions
//! write the route table, and workers that hand the sessions' events to the
//! pipeline.
//!
//! The transport belongs to the listener, as for syslog: the first worker
//! starts it and it outlives worker restarts. The sessions write the table
//! themselves, in stream order, so the workers only carry events; with none
//! wanted they idle. The collector (routers, `keep` deadlines) lives as long
//! as the transport; a new one re-arms `keep` for the routes the old one left
//! in the table.

use super::events::{Event, EventSink};
use super::peer::BgpConfig;
use super::session::{BmpConfig, Collector, CollectorConfig};
use super::stats::Stats;
use crate::InternalMetrics;
use anyhow::{Context, Result};
use async_trait::async_trait;
use flume::Receiver;
use patrolli::chan::SenderChannel;
use patrolli::config::listener::{Bgp, BgpPeering, ListenerKind};
use patrolli::payload::Scope;
use patrolli::{cache, common, config, payload};
use smol_str::SmolStr;
use socket2::{Domain, SockRef, Socket, TcpKeepalive, Type};
use std::net::{IpAddr, SocketAddr};
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
/// the servers and the expiry task have returned.
struct Running {
    rx: Receiver<Event>,
    /// Stops the intake: the servers, their sessions and the expiry task. A
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
    stats: Arc<Stats>,
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
        let bmp_socket = match &self.bgp.bmp {
            Some(bmp) => Some(
                TcpListener::bind(bmp.listener.address)
                    .await
                    .with_context(|| format!("bind bmp {}", bmp.listener.address))?,
            ),
            None => None,
        };
        let bgp_socket = match &self.bgp.bgp {
            Some(bgp) => Some(bind_bgp(bgp)?),
            None => None,
        };
        let (tx, rx) = flume::bounded(self.bgp.events.buffer_size);
        let vrfs = (!self.bgp.vrfs.is_empty()).then(|| self.bgp.vrfs.iter().cloned().collect());
        let collector = Collector::new(
            self.conf.id,
            self.conf_cache.lookup_table.clone(),
            CollectorConfig {
                bmp: self.bgp.bmp.as_ref().map(|bmp| BmpConfig {
                    keep: bmp.keep,
                    max_message_size: bmp.max_message_size,
                }),
                bgp: self.bgp.bgp.as_ref().map(BgpConfig::from),
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
        if let (Some(socket), Some(bmp)) = (bmp_socket, &self.bgp.bmp) {
            let filter = IpFilter {
                whitelist: bmp.listener.ip_whitelist.clone(),
                blacklist: bmp.listener.ip_blacklist.clone(),
            };
            tasks.spawn(
                server(
                    Transport::Bmp,
                    socket,
                    filter,
                    collector.clone(),
                    intake.clone(),
                )
                .instrument(Span::current()),
            );
            info!(transport = "bmp", addr = %bmp.listener.address, "server listening");
        }
        if let (Some(socket), Some(bgp)) = (bgp_socket, &self.bgp.bgp) {
            let filter = IpFilter {
                whitelist: bgp.listener.ip_whitelist.clone(),
                blacklist: bgp.listener.ip_blacklist.clone(),
            };
            tasks.spawn(
                server(
                    Transport::Bgp,
                    socket,
                    filter,
                    collector.clone(),
                    intake.clone(),
                )
                .instrument(Span::current()),
            );
            info!(transport = "bgp", addr = %bgp.listener.address, "server listening");
        }
        let expiry_intake = intake.clone();
        tasks.spawn(
            async move {
                // the servers going down take the intake with them
                collector.run_expiry(expiry_intake).await;
            }
            .instrument(Span::current()),
        );
        Ok(Running { rx, intake, tasks })
    }

    /// Stops the transport for good: the sessions end without arming `keep`
    /// (their routes stay until the table goes, a later transport expires
    /// them, or the listener's removal clears them: [`clear_routes`]), and
    /// the channel disconnects once they have sent their events.
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

/// Binds the passive BGP socket, with the peers' TCP-MD5 keys set before it
/// listens.
fn bind_bgp(bgp: &BgpPeering) -> Result<TcpListener> {
    let addr = bgp.listener.address;
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, None)
        .and_then(|s| {
            s.set_reuse_address(true)?;
            s.set_nonblocking(true)?;
            s.bind(&addr.into())?;
            Ok(s)
        })
        .with_context(|| format!("bind bgp {addr}"))?;
    for peer in &bgp.peers {
        let Some(password) = &peer.password else {
            continue;
        };
        // a still encrypted password is a broken invariant: the config was
        // decrypted when it was loaded
        let password = password
            .expose()
            .with_context(|| format!("bgp peer {} password", peer.address))?;
        #[cfg(target_os = "linux")]
        super::md5::set_key(&socket, peer.address, password.as_bytes())
            .with_context(|| format!("set tcp md5 key for {}", peer.address))?;
        #[cfg(not(target_os = "linux"))]
        anyhow::bail!("bgp peer {}: tcp md5 passwords need linux", peer.address);
    }
    socket
        .listen(1024)
        .with_context(|| format!("listen bgp {addr}"))?;
    TcpListener::from_std(socket.into()).with_context(|| format!("listen bgp {addr}"))
}

/// Empties the route table of `listener`, a bgp listener removed or disabled
/// for good. Its sessions are gone and no later transport of it will expire
/// their routes, so they would otherwise answer lookups forever. A listener
/// restarted by an edit keeps them instead: its new transport re-arms `keep`.
pub fn clear_routes(cache: &cache::Cache, listener: Uuid) {
    let Some(table) = cache.lookup_table.route_table_for_listener(&listener) else {
        return;
    };
    let rows = table.clear();
    if rows > 0 {
        info!(
            listener_id = %listener,
            lookup_table_id = %table.id(),
            rows,
            "routes of a removed listener cleared"
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum Transport {
    Bmp,
    Bgp,
}

impl Transport {
    fn as_str(self) -> &'static str {
        match self {
            Self::Bmp => "bmp",
            Self::Bgp => "bgp",
        }
    }
}

#[derive(Debug)]
struct IpFilter {
    whitelist: Vec<IpAddr>,
    blacklist: Vec<IpAddr>,
}

/// Accepts connections until `intake` is cancelled, then ends the sessions
/// and waits for them.
async fn server(
    transport: Transport,
    socket: TcpListener,
    filter: IpFilter,
    collector: Arc<Collector>,
    intake: CancellationToken,
) {
    let name = transport.as_str();
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
                    warn!(transport = name, error = %e, "accept failed");
                    continue;
                }
            },
        };
        // a dual-stack socket gives IPv4 peers as v4-mapped addresses
        let router = addr.ip().to_canonical();
        if !common::ip_allowed(&router, &filter.whitelist, &filter.blacklist) {
            debug!(transport = name, peer = %addr, "connection rejected by ip filter");
            continue;
        }
        let ka = TcpKeepalive::new()
            .with_time(Duration::from_secs(60))
            .with_interval(Duration::from_secs(20))
            .with_retries(4);
        if let Err(e) = SockRef::from(&stream).set_tcp_keepalive(&ka) {
            debug!(transport = name, peer = %addr, error = %e, "set tcp keepalive failed");
        }
        let (collector, cancel) = (collector.clone(), intake.child_token());
        match transport {
            Transport::Bmp => sessions.spawn(
                async move {
                    collector.session(stream, router, cancel).await;
                }
                .instrument(Span::current()),
            ),
            Transport::Bgp => {
                let local = stream
                    .local_addr()
                    .map_or(IpAddr::from([0, 0, 0, 0]), |a: SocketAddr| a.ip());
                sessions.spawn(
                    async move {
                        collector.bgp_session(stream, router, local, cancel).await;
                    }
                    .instrument(Span::current()),
                )
            }
        };
    }
    // Free the port, then end the sessions for good: a stopping listener arms
    // no `keep`.
    drop(socket);
    collector.close_all();
    while let Some(res) = sessions.join_next().await {
        if let Err(e) = res {
            error!(transport = name, error = %e, "session task failed");
        }
    }
    info!(transport = name, "server stopped");
}

#[async_trait]
impl crate::Listener for BgpListener {
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
        m.total_bytes = Stats::get(&self.stats.bytes);
        m.total_items = self.stats.total_messages();
        m.total_records_dropped += dropped;
        let mut protocols = Vec::with_capacity(2);
        if self.bgp.bmp.is_some() {
            protocols.push(super::Protocol::Bmp);
        }
        if self.bgp.bgp.is_some() {
            protocols.push(super::Protocol::Bgp);
        }
        let mut bgp = self.stats.snapshot(&protocols);
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
impl crate::test_util::Transport for BgpListener {
    async fn running(&self) -> bool {
        matches!(
            &*self.state.lock().await,
            State::Running(running) if !running.intake.is_cancelled()
        )
    }
}
