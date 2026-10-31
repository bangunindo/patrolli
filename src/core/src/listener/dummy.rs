// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::{InternalMetrics, Metrics};
use crate::chan::SenderChannel;
use crate::config::listener::ListenerKind;
use crate::payload::Scope;
use crate::{cache, common::now, config, payload};
use async_trait::async_trait;
use smol_str::SmolStr;
use std::borrow::Cow;
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::time::{Instant, interval};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};
use uuid::Uuid;

pub struct Dummy {
    conf: config::listener::Listener,
    channel: SenderChannel<&'static str, payload::ProcessorPayload>,
    forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
    conf_cache: cache::Cache,
    node_id: Option<Uuid>,

    internal_metrics: InternalMetrics,
}

impl Dummy {
    pub fn new(
        conf: config::listener::Listener,
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
    ) -> Self {
        Self {
            conf,
            channel,
            forwarder,
            conf_cache,
            node_id,
            internal_metrics: InternalMetrics::default(),
        }
    }

    async fn send_metric(&self, messages_per_batch: usize) {
        self.internal_metrics
            .total_items
            .fetch_add(1, Ordering::Relaxed);
        self.internal_metrics
            .total_records
            .fetch_add(messages_per_batch as u64, Ordering::Relaxed);
        let n = Instant::now();
        let event_time = now();

        let mut batch_builder = payload::LogsBuilder::new();
        batch_builder.scope(Scope {
            name: self.conf.name.clone(),
            listener_id: Some(self.conf.id),
            listener_type: Some(SmolStr::new_static(
                ListenerKind::from(&self.conf.config).into(),
            )),
            listener_name: Some(self.conf.name.clone()),
            monitor_type: None,
            node_id: config::NODE_ID.get().copied(),
        });
        for _ in 0..messages_per_batch {
            batch_builder.push_row(payload::LogRow {
                body: Some(Cow::Borrowed("dummy message")),
                observed_time: Some(event_time.to_utc()),
                severity: None,
                source_port: Some(0),
                source_ip: Some(IpAddr::from([127, 0, 0, 1])),
                attrs: &[],
            });
        }
        let batch = batch_builder.finish();
        let future = super::send_batch(super::SendBatch {
            batch,
            conf: &self.conf,
            conf_cache: &self.conf_cache,
            node_id: self.node_id,
            internal_metrics: &self.internal_metrics,
            channel: &self.channel,
            forwarder: &self.forwarder,
        });
        self.internal_metrics
            .engine_time_spent
            .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        // `send_batch` books its own engine and send-wait time; timing the
        // whole await here counted both a second time.
        future.await;
        self.internal_metrics
            .total_items_succeeded
            .fetch_add(1, Ordering::Relaxed);
    }
}

#[async_trait]
impl super::Listener for Dummy {
    #[tracing::instrument(skip_all, fields(listener_id = %self.conf.id, listener = %self.conf.name))]
    async fn listen(&self, token: CancellationToken) {
        let speed;
        let messages_per_batch;
        if let config::listener::Config::Dummy(dummy_conf) = &self.conf.config {
            speed = dummy_conf.speed;
            messages_per_batch = dummy_conf.messages_per_batch;
        } else {
            panic!("not dummy config")
        }
        self.internal_metrics
            .total_workers
            .fetch_add(1, Ordering::Relaxed);
        info!(metrics_per_sec = speed, "starting dummy metrics generator");
        let mut ticker = interval(Duration::from_nanos(1_000_000_000 / speed));
        loop {
            let n = Instant::now();
            tokio::select! {
                _ = ticker.tick() => {
                    self.internal_metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    self.send_metric(messages_per_batch).await;
                }
                _ = token.cancelled() => {
                    debug!("stopping listener");
                    break;
                }
            }
            self.internal_metrics
                .total_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        info!("listener stopped");
        self.internal_metrics
            .total_workers
            .fetch_sub(1, Ordering::Relaxed);
    }

    async fn metrics(&self) -> Metrics {
        self.internal_metrics
            .get(self.conf.id, self.conf.name.to_string())
    }
}
