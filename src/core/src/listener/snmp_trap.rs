// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::chan::SenderChannel;
use crate::common::snmp as snmp_common;
use crate::config::listener::ListenerKind;
use crate::config::snmp::SecurityModel;
use crate::listener::InternalMetrics;
use crate::payload::{LogRow, Scope, ZString};
use crate::{cache, config, payload};
use anyhow::Result;
use async_snmp::{Notification, NotificationReceiver, Oid, TrapV1Pdu, VarBind};
use async_trait::async_trait;
use bytes::Bytes;
use smol_str::{SmolStr, ToSmolStr};
use std::borrow::Cow;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::{Instant, interval};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};
use uuid::Uuid;

pub struct SnmpTrap {
    conf: config::listener::Listener,
    channel: SenderChannel<&'static str, payload::ProcessorPayload>,
    forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
    conf_cache: cache::Cache,
    node_id: Option<Uuid>,

    internal_metrics: InternalMetrics,
    receiver: TokioMutex<Option<NotificationReceiver>>,
}

impl SnmpTrap {
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
            receiver: TokioMutex::new(None),
        }
    }

    fn conf(&self) -> &config::listener::SnmpTrap {
        match &self.conf.config {
            config::listener::Config::SnmpTrap(conf) => conf,
            _ => panic!("not snmp trap config"),
        }
    }

    fn handle_varbinds<'a>(&self, varbinds: &'a [VarBind]) -> payload::Value<'a> {
        let values: Vec<payload::Value> = varbinds
            .iter()
            .map(|varbind| {
                snmp_common::process_value(&varbind.oid, &varbind.value, self.conf().use_mib)
            })
            .collect();
        payload::Value::Array(values)
    }

    fn handle_v1(
        &self,
        batcher: &mut payload::LogsBuilder,
        src_addr: SocketAddr,
        community: String,
        trap: TrapV1Pdu,
    ) {
        let attrs: Vec<(ZString, payload::Value)> = vec![
            (
                ZString::from_static("snmp.version"),
                payload::Value::String(ZString::from_ref("v1")),
            ),
            (
                ZString::from_static("snmp.community_or_username"),
                payload::Value::String(ZString::from_owned(community)),
            ),
            (
                ZString::from_static("snmp.enterprise"),
                payload::Value::String(ZString::from_smol(trap.enterprise().to_smolstr())),
            ),
            (
                ZString::from_static("snmp.agent_addr"),
                payload::Value::String(ZString::from_smol(
                    IpAddr::from(trap.agent_addr()).to_smolstr(),
                )),
            ),
            (
                ZString::from_static("snmp.generic_trap"),
                payload::Value::String(ZString::from_smol(trap.generic_trap().to_smolstr())),
            ),
            (
                ZString::from_static("snmp.specific_trap"),
                payload::Value::Int64(trap.specific_trap() as i64),
            ),
            (
                ZString::from_static("snmp.time_stamp"),
                payload::Value::Int64(trap.time_stamp() as i64),
            ),
            (
                ZString::from_static("snmp.data"),
                self.handle_varbinds(trap.varbinds()),
            ),
        ];
        let log_record = LogRow {
            observed_time: None,
            severity: None,
            body: None,
            source_ip: Some(src_addr.ip()),
            source_port: Some(src_addr.port()),
            attrs: &attrs,
        };
        batcher.push_row(log_record);
        self.internal_metrics
            .total_records
            .fetch_add(1, Ordering::Relaxed);
        self.internal_metrics
            .total_items_succeeded
            .fetch_add(1, Ordering::Relaxed);
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_v2cv3(
        &self,
        batcher: &mut payload::LogsBuilder,
        version: &str,
        src_addr: SocketAddr,
        community_or_username: String,
        uptime: u32,
        trap_oid: Oid,
        request_id: i32,
        varbinds: Vec<VarBind>,
        context_engine_id: Option<Bytes>,
        context_name: Option<Bytes>,
    ) {
        let mut attrs: Vec<(ZString, payload::Value)> = Vec::with_capacity(8);
        attrs.push((
            ZString::from_static("snmp.version"),
            payload::Value::String(ZString::from_ref(version)),
        ));
        attrs.push((
            ZString::from_static("snmp.community_or_username"),
            payload::Value::String(ZString::from_owned(community_or_username)),
        ));
        attrs.push((
            ZString::from_static("snmp.uptime"),
            payload::Value::Int64(uptime as i64),
        ));
        attrs.push((
            ZString::from_static("snmp.trap_oid"),
            payload::Value::String(ZString::from_smol(trap_oid.to_smolstr())),
        ));
        attrs.push((
            ZString::from_static("snmp.request_id"),
            payload::Value::Int64(request_id as i64),
        ));
        if let Some(context_engine_id) = &context_engine_id {
            attrs.push((
                ZString::from_static("snmp.context_engine_id"),
                payload::Value::Bytes(Cow::Borrowed(context_engine_id)),
            ));
        }
        if let Some(context_name) = &context_name {
            attrs.push((
                ZString::from_static("snmp.context_name"),
                match str::from_utf8(context_name) {
                    Ok(name) => payload::Value::String(ZString::from_ref(name)),
                    Err(_) => payload::Value::Bytes(Cow::Borrowed(context_name)),
                },
            ));
        }
        attrs.push((
            ZString::from_static("snmp.data"),
            self.handle_varbinds(&varbinds),
        ));
        let log_record = LogRow {
            observed_time: None,
            severity: None,
            body: None,
            source_ip: Some(src_addr.ip()),
            source_port: Some(src_addr.port()),
            attrs: &attrs,
        };
        batcher.push_row(log_record);
        self.internal_metrics
            .total_records
            .fetch_add(1, Ordering::Relaxed);
        self.internal_metrics
            .total_items_succeeded
            .fetch_add(1, Ordering::Relaxed);
    }

    fn handle_notification(
        &self,
        notif: Notification,
        addr: SocketAddr,
        batcher: &mut payload::LogsBuilder,
    ) {
        self.internal_metrics
            .total_items
            .fetch_add(1, Ordering::Relaxed);
        if !self.conf().listener.ip_whitelist.is_empty()
            && let Err(_) = self.conf().listener.ip_whitelist.binary_search(&addr.ip())
        {
            self.internal_metrics
                .total_items_dropped
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        if !self.conf().listener.ip_blacklist.is_empty()
            && let Ok(_) = self.conf().listener.ip_blacklist.binary_search(&addr.ip())
        {
            self.internal_metrics
                .total_items_dropped
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        match notif {
            Notification::TrapV1 {
                community, trap, ..
            } => {
                let Some(v1) = &self.conf().v1 else {
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let community = String::from_utf8_lossy(community.as_bytes()).to_string();
                if v1.communities.binary_search(&community).is_err() {
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
                self.handle_v1(batcher, addr, community, trap);
            }
            Notification::TrapV2c {
                community,
                uptime,
                trap_oid,
                varbinds,
                request_id,
                ..
            } => {
                let Some(v2c) = &self.conf().v2c else {
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let community = String::from_utf8_lossy(community.as_bytes()).to_string();
                if v2c.communities.binary_search(&community).is_err() {
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
                self.handle_v2cv3(
                    batcher, "v2c", addr, community, uptime, trap_oid, request_id, varbinds, None,
                    None,
                );
            }
            Notification::TrapV3 {
                username,
                context_engine_id,
                context_name,
                security_level: _,
                uptime,
                trap_oid,
                varbinds,
                request_id,
                ..
            } => {
                let Some(v3) = &self.conf().v3 else {
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let username = String::from_utf8_lossy(&username).to_string();
                if v3
                    .security_models
                    .binary_search_by(|model| match model {
                        SecurityModel::Usm(usm) => usm.username.as_str().cmp(&username),
                    })
                    .is_err()
                {
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
                self.handle_v2cv3(
                    batcher,
                    "v3",
                    addr,
                    username,
                    uptime,
                    trap_oid,
                    request_id,
                    varbinds,
                    Some(context_engine_id),
                    Some(context_name),
                );
            }
            Notification::InformV2c {
                community,
                uptime,
                trap_oid,
                varbinds,
                request_id,
                ..
            } => {
                let Some(v2c) = &self.conf().v2c else {
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let community = String::from_utf8_lossy(community.as_bytes()).to_string();
                if v2c.communities.binary_search(&community).is_err() {
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
                self.handle_v2cv3(
                    batcher, "v2c", addr, community, uptime, trap_oid, request_id, varbinds, None,
                    None,
                );
            }
            Notification::InformV3 {
                username,
                context_engine_id,
                context_name,
                security_level: _,
                uptime,
                trap_oid,
                varbinds,
                request_id,
                ..
            } => {
                let Some(v3) = &self.conf().v3 else {
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let username = String::from_utf8_lossy(&username).to_string();
                if v3
                    .security_models
                    .binary_search_by(|model| match model {
                        SecurityModel::Usm(usm) => usm.username.as_str().cmp(&username),
                    })
                    .is_err()
                {
                    self.internal_metrics
                        .total_items_dropped
                        .fetch_add(1, Ordering::Relaxed);
                    return;
                }
                self.handle_v2cv3(
                    batcher,
                    "v3",
                    addr,
                    username,
                    uptime,
                    trap_oid,
                    request_id,
                    varbinds,
                    Some(context_engine_id),
                    Some(context_name),
                );
            }
        }
    }

    async fn init_receiver(
        &self,
        receiver: &mut Option<NotificationReceiver>,
    ) -> Result<NotificationReceiver> {
        let mut builder =
            NotificationReceiver::builder().bind(self.conf().listener.address.to_string());
        if let Some(v3) = &self.conf().v3 {
            // A receiver with USM users must now name an acceptance policy; accepting
            // everything here keeps the pre-0.18 behaviour, and the per-notification
            // username/community allowlists below are unchanged. It is also an
            // authoritative engine (for informs), and answers DES informs with
            // its own salts, which async-snmp refuses to build without.
            builder = builder
                .accept_all_notifications()
                .authoritative_engine(snmp_common::authoritative_engine())
                .des_salt_state(snmp_common::des_salt_state());
            for security_model in &v3.security_models {
                let SecurityModel::Usm(usm) = security_model;
                builder = builder.usm_user(usm.username.clone(), |u| {
                    match usm.generate_master_keys() {
                        Some(master_keys) => u.with_master_keys(master_keys),
                        None => {
                            if usm.authentication.is_some() {
                                error!(
                                    snmp_user = %usm.username,
                                    "snmpv3 credentials unusable, falling back to noauthnopriv"
                                );
                            }
                            Ok(u)
                        }
                    }
                })?;
            }
        }
        let trap_receiver = builder.build().await?;
        receiver.replace(trap_receiver.clone());
        Ok(trap_receiver)
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

    async fn send_batch(&self, batch: &mut payload::LogsBuilder) {
        if batch.is_empty() {
            return;
        }
        let pl = batch.finish();
        super::send_batch(super::SendBatch {
            batch: pl,
            conf: &self.conf,
            conf_cache: &self.conf_cache,
            node_id: self.node_id,
            internal_metrics: &self.internal_metrics,
            channel: &self.channel,
            forwarder: &self.forwarder,
        })
        .await;
        // `finish` resets the builder (clearing the scope), so re-arm the scope
        // before the next batch starts collecting rows.
        self.set_scope(batch);
    }
}

#[async_trait]
impl super::Listener for SnmpTrap {
    #[tracing::instrument(skip_all, fields(listener_id = %self.conf.id, listener = %self.conf.name))]
    async fn listen(&self, token: CancellationToken) {
        self.internal_metrics
            .total_workers
            .fetch_add(1, Ordering::Relaxed);
        info!("starting listener");
        let receiver = {
            let mut receiver_lock = self.receiver.lock().await;
            match receiver_lock.as_ref() {
                Some(r) => r.clone(),
                None => match self.init_receiver(&mut receiver_lock).await {
                    Ok(r) => r,
                    Err(e) => {
                        error!(error = %format!("{e:#}"), "failed to start trap listener");
                        self.internal_metrics
                            .total_workers
                            .fetch_sub(1, Ordering::Relaxed);
                        return;
                    }
                },
            }
        };
        let mut batch = payload::LogsBuilder::new();
        self.set_scope(&mut batch);
        let mut batch_tick = interval(self.conf.batch.batch_timeout);
        loop {
            let n = Instant::now();
            tokio::select! {
                biased;

                _ = token.cancelled() => {
                    debug!("stopping listener");
                    break;
                }
                _ = batch_tick.tick() => {
                    self.internal_metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    self.send_batch(&mut batch).await;
                },
                res = receiver.recv() => {
                    self.internal_metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    match res {
                        Ok(received) => {
                            let n = Instant::now();
                            self.handle_notification(received.notification, received.source, &mut batch);
                            self.internal_metrics
                                .engine_time_spent
                                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                            if batch.num_items() >= self.conf.batch.batch_size {
                                self.send_batch(&mut batch).await;
                                batch_tick.reset();
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "trap receiver failed");
                            break;
                        }
                    }
                }
            }
            self.internal_metrics
                .total_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        self.send_batch(&mut batch).await;
        drop(receiver);
        info!("listener stopped");
        // Under the lock a starting worker takes, so it cannot clone the
        // receiver the last worker is releasing (and then bind a second
        // socket next to it).
        let mut receiver_lock = self.receiver.lock().await;
        let prev_worker_count = self
            .internal_metrics
            .total_workers
            .fetch_sub(1, Ordering::Relaxed);
        if prev_worker_count == 1 {
            receiver_lock.take();
        }
    }

    async fn metrics(&self) -> crate::common::internal_metrics::Listener {
        self.internal_metrics
            .get(self.conf.id, self.conf.name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listener::test_util::{self, CLOSE_LIMIT, item_for};
    use crate::listener::{Listener as _, Worker};
    use async_snmp::{
        Auth, AuthProtocol, AuthoritativeEngine, Client, DesSaltState, GenericTrap, PrivProtocol,
        TrapV1Notification, UsmConfig, Value, oid,
    };
    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;

    const AUTH: &str = "authpass123";
    const PRIV: &str = "privpass123";

    /// One row's attributes, by key, with the OTLP JSON value wrapper
    /// (`{"stringValue": ..}`) left on.
    type Row = HashMap<String, serde_json::Value>;

    /// A listener on a free port of 127.0.0.1, run by a `Worker` as
    /// `WorkerGroup` runs it, sending into a processor topic.
    struct Trap {
        listener: Arc<SnmpTrap>,
        worker: Worker,
        rows: flume::Receiver<payload::ProcessorPayload>,
        _admin: crate::chan::AdminChannel<&'static str, payload::ProcessorPayload>,
    }

    /// `config` is the listener's `config:` without `listener:`, which is
    /// filled with `listener` (`address` defaults to a free port).
    async fn start(config: &str, listener: &str, workers: usize) -> Trap {
        let address = if listener.contains("address") {
            ""
        } else {
            "address: '127.0.0.1:0', "
        };
        let mut conf: config::listener::SnmpTrap =
            serde_yaml_ng::from_str(&format!("listener: {{{address}{listener}}}\n{config}"))
                .unwrap();
        // Not `validate`, which rejects port 0; these sort what is searched.
        conf.listener.ip_whitelist.sort_unstable();
        conf.listener.ip_blacklist.sort_unstable();
        for v in [&mut conf.v1, &mut conf.v2c].into_iter().flatten() {
            v.validate().unwrap();
        }
        if let Some(v3) = &mut conf.v3 {
            v3.validate().unwrap();
        }
        let id = Uuid::now_v7();
        let conf = config::listener::Listener::new(
            id,
            "traps".into(),
            workers,
            config::Batch {
                batch_size: 1,
                batch_timeout: Duration::from_millis(20),
            },
            config::listener::Config::SnmpTrap(conf),
            false,
        );
        let (admin, sender, rows) = test_util::processor_topic(64).await;
        let listener = Arc::new(SnmpTrap::new(
            conf.clone(),
            sender,
            None,
            item_for(id),
            None,
        ));
        let worker = Worker::with_listener(conf, listener.clone());
        worker.adjust(workers).await;
        Trap {
            listener,
            worker,
            rows,
            _admin: admin,
        }
    }

    impl Trap {
        /// Where the receiver is bound, once the workers have bound it.
        async fn addr(&self) -> String {
            let deadline = Instant::now() + CLOSE_LIMIT;
            loop {
                if let Some(receiver) = &*self.listener.receiver.lock().await {
                    return receiver.local_addr().to_string();
                }
                assert!(Instant::now() < deadline, "receiver never bound");
                sleep(Duration::from_millis(10)).await;
            }
        }

        async fn metrics(&self) -> crate::common::internal_metrics::Listener {
            self.listener.metrics().await
        }

        /// Waits for `want` rows.
        async fn rows(&self, want: usize) -> Vec<Row> {
            let mut out = Vec::new();
            while out.len() < want {
                let pl = tokio::time::timeout(CLOSE_LIMIT, self.rows.recv_async())
                    .await
                    .unwrap_or_else(|_| panic!("got {} of {want} rows", out.len()))
                    .unwrap();
                let json: serde_json::Value = serde_json::from_slice(
                    pl.payload
                        .serialize_as(config::Format::JsonOtlpBatch)
                        .unwrap()[0]
                        .as_ref(),
                )
                .unwrap();
                collect_rows(&json, &mut out);
            }
            out
        }

        /// Waits until `n` notifications were counted, each dropped.
        async fn dropped(&self, n: u64) {
            let deadline = Instant::now() + CLOSE_LIMIT;
            loop {
                let m = self.metrics().await;
                if m.total_items_dropped >= n {
                    assert_eq!(m.total_items, n);
                    assert_eq!(m.total_items_succeeded, 0);
                    return;
                }
                assert!(Instant::now() < deadline, "nothing dropped");
                sleep(Duration::from_millis(10)).await;
            }
        }

        async fn close(self) {
            self.worker
                .close(crate::common::shutdown::Deadline::NONE)
                .await;
        }
    }

    fn collect_rows(value: &serde_json::Value, out: &mut Vec<Row>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(records) = map.get("logRecords").and_then(|r| r.as_array()) {
                    for record in records {
                        let attrs = record["attributes"].as_array().into_iter().flatten();
                        out.push(
                            attrs
                                .filter_map(|kv| {
                                    Some((kv["key"].as_str()?.to_owned(), kv["value"].clone()))
                                })
                                .collect(),
                        );
                    }
                    return;
                }
                map.values().for_each(|v| collect_rows(v, out));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| collect_rows(v, out)),
            _ => {}
        }
    }

    fn string<'a>(row: &'a Row, key: &str) -> &'a str {
        row.get(key)
            .and_then(|v| v["stringValue"].as_str())
            .unwrap_or_else(|| panic!("no string {key} in {row:?}"))
    }

    /// A sender's own authoritative engine: for a v3 trap the sender is
    /// authoritative, and it has its own engine ID.
    fn sender_engine(id: &[u8]) -> AuthoritativeEngine {
        AuthoritativeEngine::install(id.to_vec(), |_| Ok::<(), Infallible>(())).unwrap()
    }

    async fn v3_client(addr: &str, usm: UsmConfig, engine: Option<&[u8]>) -> Client {
        let mut builder = Client::builder(addr, usm)
            .des_salt_state(DesSaltState::install(|_| Ok::<(), Infallible>(())).unwrap());
        if let Some(id) = engine {
            builder = builder.local_authoritative_engine(sender_engine(id));
        }
        builder.connect().await.unwrap()
    }

    const ALL_VERSIONS: &str = "
v1: {communities: [public]}
v2c: {communities: [public]}
v3:
  security_models:
    - type: usm
      config:
        username: trapuser
        authentication: {protocol: sha256, passphrase: authpass123}
        privacy: {protocol: aes128, passphrase: privpass123}
    - type: usm
      config:
        username: desuser
        authentication: {protocol: sha1, passphrase: authpass123}
        privacy: {protocol: des, passphrase: privpass123}
";

    fn trapuser() -> UsmConfig {
        UsmConfig::new("trapuser")
            .auth_priv(AuthProtocol::Sha256, AUTH, PrivProtocol::Aes128, PRIV)
            .unwrap()
    }

    /// Every version on one listener. A `v3:` block used to stop the receiver
    /// from starting at all (async-snmp 0.18 wants an authoritative engine for
    /// USM users), taking v1 and v2c down with it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn v1_v2c_v3_traps_and_informs_produce_rows() {
        let t = start(ALL_VERSIONS, "", 2).await;
        let addr = t.addr().await;
        let hello = || {
            vec![async_snmp::VarBind::new(
                oid!(1, 3, 6, 1, 2, 1, 1, 1, 0),
                Value::from("hello"),
            )]
        };

        let v1 = Client::builder(addr.as_str(), Auth::v1("public"))
            .connect()
            .await
            .unwrap();
        let trap = TrapV1Notification::new(
            oid!(1, 3, 6, 1, 4, 1, 9999),
            [10, 0, 0, 1],
            GenericTrap::EnterpriseSpecific,
            42,
            99999,
            hello(),
        )
        .unwrap();
        v1.send_v1_trap(trap).await.unwrap();

        let v2c = Client::builder(addr.as_str(), Auth::v2c("public"))
            .connect()
            .await
            .unwrap();
        v2c.send_trap(&oid!(1, 3, 6, 1, 6, 3, 1, 1, 5, 1), 1, hello())
            .await
            .unwrap();
        // An inform returns once the listener has acknowledged it.
        v2c.send_inform(&oid!(1, 3, 6, 1, 6, 3, 1, 1, 5, 3), 2, hello())
            .await
            .unwrap();

        let v3 = v3_client(&addr, trapuser(), Some(b"trap-sender-engine")).await;
        v3.send_trap(&oid!(1, 3, 6, 1, 6, 3, 1, 1, 5, 2), 3, hello())
            .await
            .unwrap();
        // An inform's receiver is the authoritative engine: patrolli's own.
        let v3 = v3_client(&addr, trapuser(), None).await;
        v3.send_inform(&oid!(1, 3, 6, 1, 6, 3, 1, 1, 5, 4), 4, hello())
            .await
            .unwrap();

        let mut rows = t.rows(5).await;
        rows.sort_by_key(|r| {
            r.get("snmp.uptime")
                .or(r.get("snmp.time_stamp"))
                .map(|v| v["intValue"].to_string())
        });
        let seen: Vec<(&str, &str)> = rows
            .iter()
            .map(|r| {
                (
                    string(r, "snmp.version"),
                    string(r, "snmp.community_or_username"),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("v2c", "public"),
                ("v2c", "public"),
                ("v3", "trapuser"),
                ("v3", "trapuser"),
                ("v1", "public"),
            ]
        );
        let v1_row = &rows[4];
        assert_eq!(string(v1_row, "snmp.enterprise"), "1.3.6.1.4.1.9999");
        assert_eq!(string(v1_row, "snmp.agent_addr"), "10.0.0.1");
        assert_eq!(string(v1_row, "snmp.generic_trap"), "enterpriseSpecific");
        let trap_oids: Vec<&str> = rows[..4]
            .iter()
            .map(|r| string(r, "snmp.trap_oid"))
            .collect();
        assert_eq!(
            trap_oids,
            [
                "1.3.6.1.6.3.1.1.5.1",
                "1.3.6.1.6.3.1.1.5.3",
                "1.3.6.1.6.3.1.1.5.2",
                "1.3.6.1.6.3.1.1.5.4",
            ]
        );
        for row in &rows {
            assert_eq!(string(row, "snmp.data.0.oid"), "1.3.6.1.2.1.1.1.0");
            assert_eq!(string(row, "snmp.data.0.value"), "hello");
        }
        let m = t.metrics().await;
        assert_eq!((m.total_items, m.total_items_succeeded), (5, 5));
        t.close().await;
    }

    /// DES needs a salt state on both ends, which async-snmp 0.18 refuses to
    /// build a receiver without.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn v3_des_authpriv_trap_decrypts() {
        let t = start(ALL_VERSIONS, "", 1).await;
        let addr = t.addr().await;
        let usm = UsmConfig::new("desuser")
            .auth_priv(AuthProtocol::Sha1, AUTH, PrivProtocol::Des, PRIV)
            .unwrap();
        let v3 = v3_client(&addr, usm, Some(b"des-sender-engine")).await;
        v3.send_trap(&oid!(1, 3, 6, 1, 6, 3, 1, 1, 5, 1), 1, vec![])
            .await
            .unwrap();
        let rows = t.rows(1).await;
        assert_eq!(string(&rows[0], "snmp.community_or_username"), "desuser");
        t.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn non_utf8_context_name_is_bytes() {
        let t = start(ALL_VERSIONS, "", 1).await;
        let addr = t.addr().await;
        let usm = trapuser().context_name(vec![0xff, 0xfe]);
        let v3 = v3_client(&addr, usm, Some(b"ctx-sender-engine")).await;
        v3.send_trap(&oid!(1, 3, 6, 1, 6, 3, 1, 1, 5, 1), 1, vec![])
            .await
            .unwrap();
        let rows = t.rows(1).await;
        let context = &rows[0]["snmp.context_name"];
        assert!(context.get("bytesValue").is_some(), "not bytes: {context}");
        t.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn dropped_notifications_are_counted() {
        let v2c_trap = async |addr: String, community: &str| {
            Client::builder(addr.as_str(), Auth::v2c(community))
                .connect()
                .await
                .unwrap()
                .send_trap(&oid!(1, 3, 6, 1, 6, 3, 1, 1, 5, 1), 1, vec![])
                .await
                .unwrap();
        };
        let v2c_only = "v2c: {communities: [public]}";

        // wrong community
        let t = start(v2c_only, "", 1).await;
        v2c_trap(t.addr().await, "private").await;
        t.dropped(1).await;
        t.close().await;

        // a version the listener does not take
        let t = start(v2c_only, "", 1).await;
        let v1 = Client::builder(t.addr().await.as_str(), Auth::v1("public"))
            .connect()
            .await
            .unwrap();
        v1.send_trap(&oid!(1, 3, 6, 1, 6, 3, 1, 1, 5, 1), 1, vec![])
            .await
            .unwrap();
        t.dropped(1).await;
        t.close().await;

        // a source the whitelist leaves out
        let t = start(v2c_only, "ip_whitelist: ['192.0.2.1']", 1).await;
        v2c_trap(t.addr().await, "public").await;
        t.dropped(1).await;
        t.close().await;

        // a blacklisted source
        let t = start(v2c_only, "ip_blacklist: ['127.0.0.1']", 1).await;
        v2c_trap(t.addr().await, "public").await;
        t.dropped(1).await;
        t.close().await;
    }

    /// #18: a worker that cannot bind returns, and leaves the worker count as
    /// it found it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bind_failure_decrements_workers() {
        let taken = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = taken.local_addr().unwrap();
        let t = start(
            "v2c: {communities: [public]}",
            &format!("address: '{addr}'"),
            1,
        )
        .await;
        let deadline = Instant::now() + CLOSE_LIMIT;
        loop {
            let finished = {
                let units = t.worker.units.lock().await;
                units.live.iter().all(|unit| unit.handle.is_finished())
            };
            if finished {
                break;
            }
            assert!(Instant::now() < deadline, "worker kept running");
            sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(t.metrics().await.total_workers, 0);
        assert!(t.listener.receiver.lock().await.is_none());
        t.close().await;
    }

    /// The workers share one socket, released with the last of them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn receiver_released_when_last_worker_exits() {
        let t = start("v2c: {communities: [public]}", "", 2).await;
        let addr = t.addr().await;
        t.worker.adjust(1).await;
        assert!(t.listener.receiver.lock().await.is_some());
        t.worker.adjust(0).await;
        assert_eq!(t.metrics().await.total_workers, 0);
        assert!(t.listener.receiver.lock().await.is_none());
        std::net::UdpSocket::bind(addr.as_str()).expect("port released");
        t.close().await;
    }
}
