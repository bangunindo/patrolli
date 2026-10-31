// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::v1::{
    ForwardStatus, PayloadBatch, SignalType,
    inter_cluster_service_client::InterClusterServiceClient,
};
use crate::common::internal_metrics::NodeMetrics;
use crate::{config, payload};
use anyhow::Result;
use flume::Receiver;
use otel_arrow_dfe_pdata::{OtapArrowRecords, Producer};
use prost::Message;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tokio::time::{MissedTickBehavior, interval};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{Request, transport::Channel};
use tracing::{info, warn};

pub(super) struct ForwarderClient {
    forward_receiver: Receiver<payload::Payload>,
    token: CancellationToken,
    transport: Channel,
    conf: config::cluster::Forwarder,
    bearer: super::comm::Bearer,
    metrics: Arc<NodeMetrics>,

    worker_count: AtomicUsize,
    /// How many workers currently hold a live stream to the peer. 0 means
    /// disconnected: nobody drains the rendezvous forward channel, so the
    /// receiver bridge drops payloads for this node instead of blocking.
    streams: watch::Sender<usize>,
    done: CancellationToken,
}

/// Counts a live stream in [`ForwarderClient::streams`] (and mirrors it into
/// the node's `total_connections` metric) for as long as it is held, covering
/// every exit path of `forward_loop`.
struct StreamGuard<'a>(&'a watch::Sender<usize>, &'a AtomicUsize);

impl<'a> StreamGuard<'a> {
    fn new(streams: &'a watch::Sender<usize>, connections: &'a AtomicUsize) -> Self {
        streams.send_modify(|live| *live += 1);
        connections.fetch_add(1, Ordering::Relaxed);
        Self(streams, connections)
    }
}

impl Drop for StreamGuard<'_> {
    fn drop(&mut self) {
        self.0.send_modify(|live| *live -= 1);
        self.1.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Counts a running worker in [`ForwarderClient::worker_count`]; the last one
/// out cancels `done`, which `close` waits for. A guard, so a worker that
/// panics still leaves, and `close` does not wait on it forever.
struct WorkerGuard<'a>(&'a ForwarderClient);

impl<'a> WorkerGuard<'a> {
    fn new(client: &'a ForwarderClient) -> Self {
        client.worker_count.fetch_add(1, Ordering::Relaxed);
        Self(client)
    }
}

impl Drop for WorkerGuard<'_> {
    fn drop(&mut self) {
        if self.0.worker_count.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.0.done.cancel();
        }
    }
}

/// Books the payloads still queued in a flush that just aborted. They were
/// counted in `total_records` on receipt and go down with the stream, so
/// without this they would vanish from the sent/errored split.
fn err_remaining(metrics: &NodeMetrics, pending: impl Iterator<Item = payload::Payload>) {
    let items: u64 = pending.map(|pl| pl.num_items() as u64).sum();
    if items > 0 {
        metrics
            .total_records_errored
            .fetch_add(items, Ordering::Relaxed);
    }
}

impl ForwarderClient {
    pub(super) fn new(
        forward_receiver: Receiver<payload::Payload>,
        token: CancellationToken,
        transport: Channel,
        conf: config::cluster::Forwarder,
        bearer: super::comm::Bearer,
        metrics: Arc<NodeMetrics>,
    ) -> Arc<Self> {
        Arc::new(Self {
            forward_receiver,
            token,
            transport,
            conf,
            bearer,
            metrics,
            worker_count: AtomicUsize::new(0),
            streams: watch::Sender::new(0),
            done: CancellationToken::new(),
        })
    }

    pub(super) fn connection_watch(&self) -> watch::Receiver<usize> {
        self.streams.subscribe()
    }

    async fn forward_loop(&self, retry_delay: &mut Duration) -> Result<()> {
        let (request_tx, request_rx) = mpsc::channel::<PayloadBatch>(1);
        let mut client = InterClusterServiceClient::new(self.transport.clone());
        let mut request = Request::new(ReceiverStream::new(request_rx));
        self.bearer.apply(&mut request);
        let mut acks = client.forward_payloads(request).await?.into_inner();
        let _live = StreamGuard::new(&self.streams, &self.metrics.total_connections);

        let mut logs = Producer::new();
        let mut metrics = Producer::new();
        let mut traces = Producer::new();
        let mut batch = payload::PayloadBatch::default();
        let mut batch_tick = interval(self.conf.batch.batch_timeout);
        batch_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

        let mut payload_buffer = bytes::BytesMut::new();
        let mut survive_one_iter = false;
        let err_metrics = |items: u64, bytes: u64| {
            self.metrics
                .total_records_errored
                .fetch_add(items, Ordering::Relaxed);
            self.metrics
                .total_bytes_errored
                .fetch_add(bytes, Ordering::Relaxed);
        };
        loop {
            let n_iter = Instant::now();
            let mut sender_dropped = false;
            let flush = tokio::select! {
                _ = self.token.cancelled() => return Ok(()),
                // Nothing is in flight here, so the ack stream moving means the
                // peer ended the call (a node shutting down ends them) or the
                // connection broke. Reconnect now: a dead stream is otherwise
                // only found by losing the next batch on it.
                ack = acks.message() => {
                    err_metrics(batch.num_items() as u64, 0);
                    match ack {
                        Ok(Some(_)) => anyhow::bail!("unexpected ack from peer"),
                        Ok(None) => anyhow::bail!("peer closed ack stream"),
                        Err(e) => return Err(e.into()),
                    }
                }
                _ = batch_tick.tick() => !batch.is_empty(),
                payload = self.forward_receiver.recv_async() => match payload {
                    Ok(payload) => {
                        self.metrics
                            .total_records
                            .fetch_add(payload.num_items() as u64, Ordering::Relaxed);
                        batch.push(payload);
                        batch.num_items() >= self.conf.batch.batch_size
                    }
                    // local sender was dropped; flush what's pending, then exit
                    Err(_) => {
                        sender_dropped = true;
                        !batch.is_empty()
                    }
                }
            };
            self.metrics
                .receive_wait_time
                .fetch_add(n_iter.elapsed().as_nanos() as u64, Ordering::Relaxed);
            if flush {
                let mut pending = batch.batch(Some(self.conf.batch.batch_size)).into_iter();
                while let Some(mut payload) = pending.next() {
                    let items = payload.num_items() as u64;
                    let n = Instant::now();
                    let encoded = (|| {
                        let (bar, signal) = match &mut payload.data {
                            OtapArrowRecords::Logs(_) => {
                                (logs.produce_bar(&mut payload.data)?, SignalType::Logs)
                            }
                            OtapArrowRecords::Metrics(_) => {
                                (metrics.produce_bar(&mut payload.data)?, SignalType::Metrics)
                            }
                            OtapArrowRecords::Traces(_) => {
                                (traces.produce_bar(&mut payload.data)?, SignalType::Traces)
                            }
                        };
                        payload_buffer.reserve(bar.encoded_len());
                        bar.encode(&mut payload_buffer)?;
                        Ok::<_, anyhow::Error>(signal)
                    })();
                    self.metrics
                        .engine_time_spent
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    let signal = match encoded {
                        Ok(signal) => signal,
                        Err(e) => {
                            err_metrics(items, 0);
                            err_remaining(&self.metrics, pending.by_ref());
                            return Err(e);
                        }
                    };
                    let payload = payload_buffer.split().freeze();
                    let bytes = payload.len() as u64;
                    self.metrics.total_bytes.fetch_add(bytes, Ordering::Relaxed);
                    let payload = PayloadBatch {
                        signal: signal as i32,
                        otap_payload: payload,
                    };
                    let n = Instant::now();
                    tokio::select! {
                        _ = self.token.cancelled() => return Ok(()),
                        result = request_tx.send(payload) => {
                            if result.is_err() {
                                err_metrics(items, bytes);
                                err_remaining(&self.metrics, pending.by_ref());
                                anyhow::bail!("forward request stream closed");
                            }
                        }
                    };
                    let ack = tokio::select! {
                        _ = self.token.cancelled() => return Ok(()),
                        ack = acks.message() => match ack {
                            Ok(Some(ack)) => ack,
                            Ok(None) => {
                                err_metrics(items, bytes);
                                err_remaining(&self.metrics, pending.by_ref());
                                anyhow::bail!("peer closed ack stream");
                            }
                            Err(e) => {
                                err_metrics(items, bytes);
                                err_remaining(&self.metrics, pending.by_ref());
                                return Err(e.into());
                            }
                        },
                    };
                    self.metrics
                        .send_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);

                    if ack.status != ForwardStatus::Ok as i32 {
                        err_metrics(items, bytes);
                        err_remaining(&self.metrics, pending.by_ref());
                        anyhow::bail!("peer rejected forwarded payload: {}", ack.message);
                    };
                    self.metrics
                        .total_records_sent
                        .fetch_add(items, Ordering::Relaxed);
                    self.metrics
                        .total_bytes_sent
                        .fetch_add(bytes, Ordering::Relaxed);
                }
                batch_tick.reset();
                // survived one full flush, reset retry delay
                if !survive_one_iter {
                    survive_one_iter = true;
                    // the delay only grows past its floor after a failed attempt
                    if *retry_delay > Duration::from_secs(1) {
                        info!("forward stream recovered");
                    }
                    *retry_delay = Duration::from_secs(1);
                };
            }
            self.metrics
                .total_time_spent
                .fetch_add(n_iter.elapsed().as_nanos() as u64, Ordering::Relaxed);
            if sender_dropped {
                return Ok(());
            }
        }
    }

    pub(super) async fn close(&self) {
        self.token.cancel();
        self.done.cancelled().await;
    }

    pub(super) async fn run(&self) {
        let _worker = WorkerGuard::new(self);
        let mut retry_delay = Duration::from_secs(1);
        while !self.token.is_cancelled() {
            match self.forward_loop(&mut retry_delay).await {
                Ok(()) if self.token.is_cancelled() || self.forward_receiver.is_disconnected() => {
                    break;
                }
                Ok(()) => warn!(retry_delay = ?retry_delay, "forward stream closed, reconnecting"),
                Err(err) => {
                    warn!(error = %format!("{err:#}"), retry_delay = ?retry_delay, "forward loop failed");
                }
            }
            tokio::select! {
                _ = self.token.cancelled() => break,
                _ = tokio::time::sleep(retry_delay) => {}
            }
            retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
        }
    }
}
