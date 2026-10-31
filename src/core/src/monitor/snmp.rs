// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::MonitorRecord;
use crate::cache;
use crate::cache::item::Stats;
use crate::common::snmp as snmp_common;
use crate::config::endpoint::{Config as EndpointConfig, Endpoint};
use crate::config::host::Host;
use crate::config::item::{Config as ItemConfig, Item, ItemKind, Snmp, SnmpWalk};
use crate::config::snmp::{ChainStrategy, OidChain};
use crate::payload;
use crate::payload::ZStringOwned;
use anyhow::Context;
use async_recursion::async_recursion;
use async_snmp::{
    Auth, Client, DEFAULT_MAX_OIDS_PER_REQUEST, EngineCache, Error as SnmpError, ErrorStatus, Oid,
    Retry, UdpHandle, UdpTransport, Value, WalkMethod, WalkOptions, WalkStream,
};
use chrono::Utc;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use smol_str::ToSmolStr;
use std::collections::{BTreeMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::mem::discriminant;
use std::net::SocketAddr;
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::lookup_host;
use tokio::sync::{Mutex as TokioMutex, RwLock as TokioRwLock};
use tokio::task::JoinSet;
use tracing::{Instrument, Span, debug, error, trace};
use uuid::Uuid;

/// Translate `disable_bulk` into the walk operation it selects. `false` forces
/// GETBULK rather than leaving the choice to the SNMP version, which is what the
/// pre-0.18 `WalkMode` mapping did.
fn walk_method(disable_bulk: Option<bool>) -> WalkMethod {
    match disable_bulk {
        None => WalkMethod::Auto,
        Some(true) => WalkMethod::GetNext,
        Some(false) => WalkMethod::GetBulk,
    }
}

/// Classify an SNMP failure into the kind the stats page shows, which is also
/// what decides whether the check is retried.
fn map_snmp_err(e: &SnmpError) -> super::Error {
    match e {
        SnmpError::Timeout { .. } => super::Error::timeout("request timeout"),
        // A closed transport never delivers again, but a fresh one is built on
        // the next check, so this is still worth coming back to.
        SnmpError::Network { .. } | SnmpError::Closed { .. } => {
            super::Error::unreachable(e.to_string())
        }
        SnmpError::Auth { .. } => super::Error::auth(e.to_string()),
        SnmpError::MalformedResponse { .. } | SnmpError::WalkAborted { .. } => {
            super::Error::malformed(e.to_string())
        }
        SnmpError::Config(_) | SnmpError::InvalidOid(_) => super::Error::config(e.to_string()),
        // The object isn't visible to us. Fixing it means changing the item's OID
        // or the agent's VACM view — neither happens within a retry window, so
        // don't spend three round trips rediscovering that.
        SnmpError::Snmp {
            status:
                ErrorStatus::NoSuchName | ErrorStatus::NoAccess | ErrorStatus::AuthorizationError,
            ..
        } => super::Error::not_supported(e.to_string()),
        // The agent has something to say about our request. TooBig is often fixed
        // by fewer bulk repetitions, GenErr is usually transient.
        SnmpError::Snmp { .. } => super::Error::target(e.to_string()),
        // `SnmpError` is non-exhaustive; a variant added upstream is the library
        // telling us something we don't model yet, so treat it as retryable
        // rather than silently classing it as a config problem.
        _ => super::Error::target(e.to_string()),
    }
}

static UDP_TRANSPORT: OnceLock<Vec<UdpTransport>> = OnceLock::new();
static ENGINE_CACHE: LazyLock<Arc<EngineCache>> = LazyLock::new(|| Arc::new(EngineCache::new()));

/// `get_udp_transport` binds its sockets once per process, on whichever
/// runtime asks first, and they close when that runtime does. A
/// `#[tokio::test]` runtime per test would leave every test after the first
/// one with a dead transport, so every test that builds a client runs on this
/// one.
#[cfg(test)]
pub(super) static TEST_RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
});

fn udp_select_shard(shards: &'static [UdpTransport], dest: SocketAddr) -> &'static UdpTransport {
    let mut hasher = FxHasher::default();
    dest.hash(&mut hasher);
    let shard_idx = hasher.finish() as usize % shards.len();
    &shards[shard_idx]
}

async fn get_udp_transport(dest: SocketAddr) -> &'static UdpTransport {
    match UDP_TRANSPORT.get() {
        Some(transport) => udp_select_shard(transport, dest),
        None => {
            // some race condition is expected here, set might fail if another thread already set it
            let default_shards = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(8);
            let mut transports = Vec::with_capacity(default_shards);
            for _ in 0..default_shards {
                let udp_transport = UdpTransport::builder().bind("0.0.0.0:0").build().await;
                if let Err(e) = &udp_transport {
                    error!(error = %e, "failed to bind snmp udp transport");
                }
                let udp_transport = udp_transport.unwrap();
                transports.push(udp_transport);
            }
            let _ = UDP_TRANSPORT.set(transports);
            udp_select_shard(UDP_TRANSPORT.get().unwrap(), dest)
        }
    }
}

/// Helper function to build a child OID from a base OID and a value (Integer or IpAddress) or Oid
fn build_child_oid(
    base_oid: &Oid,
    strategy: &ChainStrategy,
    last_arc: &Oid,
    value: &Value,
) -> Option<Oid> {
    match (strategy, value) {
        (ChainStrategy::Value, Value::Integer(val)) => Some(base_oid.clone().child(*val as u32)),
        (ChainStrategy::Value, Value::IpAddress(val)) => Some(
            base_oid
                .clone()
                .child(val[0] as u32)
                .child(val[1] as u32)
                .child(val[2] as u32)
                .child(val[3] as u32),
        ),
        (ChainStrategy::OidPosition(pos), _) => pos
            .apply(last_arc)
            .map(|sub| Oid::new(base_oid.arcs().iter().chain(sub.arcs()).copied())),
        (ChainStrategy::OidPositionReversed(pos), _) => pos
            .apply_reversed(last_arc)
            .map(|sub| Oid::new(base_oid.arcs().iter().chain(sub.arcs()).copied())),
        _ => None,
    }
}

/// One GET for a batch of `snmp` items on the same endpoint.
///
/// A failure of the request itself (timeout, auth, network) fails every item. An
/// OID the agent doesn't have fails only the items asking for it, reported through
/// [`super::Collected::failed`], so one bad OID can't take its batch neighbours down.
pub async fn process_snmp(
    units: &[cache::item::Entry],
    endpoint: &Endpoint,
    host: &Host,
    timeout: Duration,
) -> Result<super::Collected, super::Error> {
    let n_start = Instant::now();
    let mut items: Vec<(&cache::item::Entry, &Snmp)> = Vec::with_capacity(units.len());
    // Deduplicated, so two items on the same OID share one varbind.
    let mut pending: Vec<Oid> = Vec::with_capacity(units.len());
    for unit in units {
        // The scheduler batches by item kind, so this is a broken invariant.
        let ItemConfig::Snmp(config) = &unit.item.config else {
            return Err(super::Error::internal("not an snmp config"));
        };
        items.push((unit, config));
        if !pending.contains(&config.oid) {
            pending.push(config.oid.clone());
        }
    }
    if pending.is_empty() {
        return Ok(super::Collected::empty());
    }
    // further retry is handled by upstream
    let client = snmp_client(endpoint, timeout, 0, None).await?;
    let mut values: FxHashMap<Oid, Value> = FxHashMap::default();
    let mut unsupported: FxHashSet<Oid> = FxHashSet::default();
    while !pending.is_empty() {
        let e = match client.get_many(&pending).await {
            Ok(response) => {
                // Keyed by OID rather than position: a compatible-mode response may
                // come back reordered or with extra bindings (see `anomalies`).
                values.extend(response.varbinds.into_iter().map(|vb| (vb.oid, vb.value)));
                break;
            }
            Err(e) => e,
        };
        // SNMPv1 has no per-varbind exceptions: one OID the agent doesn't have
        // fails the whole PDU with noSuchName and points at it. Set it aside and
        // ask again for the rest. Every pass removes an OID, so this ends.
        let Some(culprit) = no_such_name_culprit(&e, &pending) else {
            return Err(map_snmp_err(&e));
        };
        pending.retain(|oid| oid != &culprit);
        unsupported.insert(culprit);
    }
    let (rows, failed) = collect_rows(
        &items,
        endpoint,
        host,
        client.peer_addr(),
        &values,
        &unsupported,
    );
    debug!(
        elapsed = ?n_start.elapsed(),
        rows = rows.len(),
        failed = failed.len(),
        "snmp get finished"
    );
    let records = rows.len() as u64;
    let record = if rows.is_empty() {
        MonitorRecord::Null
    } else {
        MonitorRecord::BatchLog(rows)
    };
    Ok(super::Collected::partial(record, records, failed))
}

/// The OID a noSuchName error blames, if it is one we asked for.
///
/// Prefers the OID the library resolved; falls back to the 1-based error index,
/// which is only positional while the request fit in one PDU — `get_many` splits
/// past `DEFAULT_MAX_OIDS_PER_REQUEST`, and the index is then chunk-relative.
fn no_such_name_culprit(e: &SnmpError, pending: &[Oid]) -> Option<Oid> {
    let SnmpError::Snmp {
        status: ErrorStatus::NoSuchName,
        index,
        oid,
        ..
    } = e
    else {
        return None;
    };
    let culprit = match oid {
        Some(oid) => Some(oid.as_ref().clone()),
        None if pending.len() <= DEFAULT_MAX_OIDS_PER_REQUEST => (*index as usize)
            .checked_sub(1)
            .and_then(|idx| pending.get(idx))
            .cloned(),
        None => None,
    };
    culprit.filter(|oid| pending.contains(oid))
}

/// Turn a GET's answers into one row per item, or a failure for the items whose
/// OID didn't answer.
fn collect_rows(
    items: &[(&cache::item::Entry, &Snmp)],
    endpoint: &Endpoint,
    host: &Host,
    peer: SocketAddr,
    values: &FxHashMap<Oid, Value>,
    unsupported: &FxHashSet<Oid>,
) -> (Vec<super::MonitorLog>, FxHashMap<Uuid, super::Error>) {
    let mut rows = Vec::with_capacity(items.len());
    let mut failed = FxHashMap::default();
    let observed_time = Utc::now();
    for (unit, config) in items {
        if unsupported.contains(&config.oid) {
            failed.insert(unit.item.id, super::Error::not_supported("no such name"));
            continue;
        }
        let Some(value) = values.get(&config.oid) else {
            failed.insert(
                unit.item.id,
                super::Error::malformed("agent returned no binding for the oid"),
            );
            continue;
        };
        // noSuchObject/noSuchInstance: the v2c+ form of noSuchName.
        if value.is_exception() {
            failed.insert(unit.item.id, super::Error::not_supported(value.to_string()));
            continue;
        }
        let data = snmp_common::process_value(&config.oid, value, config.use_mib).into_owned();
        let row = payload::LogRowOwned {
            source_ip: Some(peer.ip()),
            source_port: Some(peer.port()),
            observed_time: Some(observed_time),
            severity: None,
            body: None,
            attrs: vec![
                (
                    ZStringOwned::from_static("snmp.oid"),
                    config.oid.to_smolstr().into(),
                ),
                (ZStringOwned::from_static("snmp.data"), vec![data].into()),
            ],
        };
        rows.push(super::MonitorLog {
            row,
            resource: super::create_resource(host, Some(endpoint), &unit.item),
            kind: ItemKind::from(&unit.item.config),
            processors: unit.item.processors.clone(),
            stats: unit.stats.clone(),
        });
    }
    (rows, failed)
}

struct WalkOp {
    oid_chain: OidChain,
    walk: TokioMutex<WalkStream<UdpHandle>>,
    client: Client,
    chain_cache: Arc<TokioRwLock<FxHashMap<Oid, (Value, payload::ValueOwned)>>>,
}

struct ChunkResult {
    op: Arc<WalkOp>,
    values: Vec<(Oid, payload::ValueOwned)>,
    last_arc: Option<Oid>,
    finished: bool,
}

#[async_recursion]
async fn process_chain(
    op: &WalkOp,
    last_arc: Oid,
    chain: &OidChain,
    varbind_value: &Value,
    values: &mut Vec<(Oid, payload::ValueOwned)>,
    use_mib: bool,
) {
    let Some(child_oid) =
        build_child_oid(&chain.oid, &chain.chain_strategy, &last_arc, varbind_value)
    else {
        return;
    };
    let chain_cache = op.chain_cache.read().await;
    let using_cache;
    let value = if let Some(chain_value) = chain_cache.get(&child_oid).cloned() {
        drop(chain_cache);
        using_cache = true;
        chain_value
    } else {
        drop(chain_cache);
        let response = match op.client.get(&child_oid).await {
            Ok(response) => response,
            Err(e) => {
                trace!(oid = %child_oid, error = %e, "chained get failed");
                return;
            }
        };
        // The agent may answer with no binding at all, or with more than we asked
        // for; either way there is no single value to chain on.
        let Some(varbind) = response.single() else {
            return;
        };
        using_cache = false;
        let value = snmp_common::process_value(&child_oid, &varbind.value, use_mib).into_owned();
        (varbind.value.clone(), value)
    };
    let varbind_value = value.0.clone();
    // An agent that answers noSuchObject/noSuchInstance/endOfMibView has told us the
    // column isn't there, so the row is short one value — same as a chain we couldn't
    // build or couldn't fetch, and that is what `max_unknown_oids_per_row` counts.
    // `process_value` would otherwise fold the exception into a plain `value: null`
    // and it would read as a column that answered. Still cached, so a wide table
    // doesn't re-ask for the same missing oid once per row.
    let is_exception = varbind_value.is_exception();
    if !is_exception {
        values.push((last_arc.clone(), value.1.clone()));
    }
    if !using_cache {
        op.chain_cache.write().await.insert(child_oid, value);
    }
    if is_exception {
        return;
    }
    for chain in chain.chain.iter() {
        process_chain(op, last_arc.clone(), chain, &varbind_value, values, use_mib).await;
    }
}

async fn read_walk_chunk(
    op: Arc<WalkOp>,
    chunk_size: usize,
    use_mib: bool,
) -> Result<ChunkResult, super::Error> {
    let mut values = Vec::with_capacity(chunk_size);
    let mut finished = false;

    let mut walk = op.walk.lock().await;
    let mut last_arc: Option<Oid> = None;

    trace!(oid = %op.oid_chain.oid, "walking");
    for _ in 0..chunk_size {
        let varbind = match walk.next().await {
            Some(Ok(varbind)) => varbind,
            Some(Err(e)) => {
                let err = map_snmp_err(&e);
                trace!(error = %err, "walk error");
                return Err(err);
            }
            None => {
                finished = true;
                break;
            }
        };
        trace!(oid = %varbind.oid, "got varbind");
        let oid_suffix = varbind
            .oid
            .strip_prefix(&op.oid_chain.oid)
            .ok_or_else(|| super::Error::malformed("walked oid is not under the requested root"))?;
        values.push((
            oid_suffix.clone(),
            snmp_common::process_value(&varbind.oid, &varbind.value, use_mib).into_owned(),
        ));
        for chain in op.oid_chain.chain.iter() {
            process_chain(
                &op,
                oid_suffix.clone(),
                chain,
                &varbind.value,
                &mut values,
                use_mib,
            )
            .await;
        }
        last_arc = Some(oid_suffix);
    }
    drop(walk);
    Ok(ChunkResult {
        op,
        values,
        finished,
        last_arc,
    })
}

fn get_min_last_arc<'a>(walk_entries: impl Iterator<Item = &'a Option<Oid>>) -> Option<Oid> {
    let mut min_last_arc: Option<Oid> = None;
    for e in walk_entries {
        if e.is_none() {
            return None;
        }
        match (&min_last_arc, e.as_ref()) {
            (None, Some(e)) => {
                min_last_arc = Some(e.clone());
            }
            (Some(last_arc), Some(e)) if e < last_arc => {
                min_last_arc = Some(e.clone());
            }
            _ => {}
        }
    }
    min_last_arc
}

fn row_is_incomplete(oid_count: usize, max_unknown: Option<usize>, values: usize) -> bool {
    max_unknown.is_some_and(|max_unknown| oid_count.saturating_sub(values) > max_unknown)
}

// Runs inside the monitor worker's `process_item` span, which carries the item identity.
async fn snmp_walk(
    client: Client,
    item_conf: &Item,
    endpoint: &Endpoint,
    host: &Host,
    snmp_conf: &SnmpWalk,
    stats: &Arc<Stats>,
    sender: &flume::Sender<MonitorRecord>,
) -> Result<super::Collected, super::Error> {
    debug!("processing snmp walk item");
    let n_start = Instant::now();
    // Rows, not data points. Counts everything this walk produced, including the
    // rows streamed straight to the batcher below and therefore absent from the
    // returned record — which is why it has to be reported back explicitly.
    let mut total_rows = 0u64;
    // Rows discarded for answering too few of the configured oids. Charged to the item
    // as dropped at the end, so a table that mostly went missing doesn't read as a
    // device that simply returned nothing.
    let mut total_dropped = 0u64;
    let oid_count = snmp_conf.oid_count();
    let max_unknown_oid = snmp_conf.max_unknown_oids_per_row;
    let chain_cache = Arc::new(TokioRwLock::new(FxHashMap::default()));
    let mut last_oid_data: BTreeMap<Oid, Option<Oid>> = snmp_conf
        .oids
        .iter()
        .map(|oid_chain| (oid_chain.oid.clone(), None))
        .collect();
    let mut result = BTreeMap::<Oid, Vec<payload::ValueOwned>>::default();
    let get_row = |attrs| payload::LogRowOwned {
        source_ip: Some(client.peer_addr().ip()),
        source_port: Some(client.peer_addr().port()),
        observed_time: Some(Utc::now()),
        severity: None,
        body: None,
        attrs,
    };
    let resource = super::create_resource(host, Some(endpoint), item_conf);
    let walk_entries = snmp_conf
        .oids
        .iter()
        .map(|oid_chain| {
            let walk = client.walk(oid_chain.oid.clone()).map_err(|e| {
                super::Error::config(format!("cannot walk oid {}: {e}", oid_chain.oid))
            })?;

            Ok(Arc::new(WalkOp {
                oid_chain: oid_chain.clone(),
                walk: TokioMutex::new(walk),
                client: client.clone(),
                chain_cache: chain_cache.clone(),
            }))
        })
        .collect::<Result<Vec<_>, super::Error>>()?;
    let mut walk_ops = walk_entries.iter().cloned().collect::<VecDeque<_>>();
    if walk_entries.is_empty() {
        return Ok(super::Collected::empty());
    }
    let mut in_flight = JoinSet::new();
    let span = Span::current();
    while !walk_ops.is_empty() || !in_flight.is_empty() {
        while in_flight.len() < snmp_conf.poll_worker_count && !walk_ops.is_empty() {
            let op = walk_ops.pop_front().unwrap();
            in_flight.spawn(
                read_walk_chunk(op, snmp_conf.chunk_size, snmp_conf.use_mib)
                    .instrument(span.clone()),
            );
        }

        let Some(joined) = in_flight.join_next().await else {
            break;
        };
        let chunk =
            joined.map_err(|e| super::Error::internal(format!("walk task panicked: {e}")))??;
        last_oid_data.insert(chunk.op.oid_chain.oid.clone(), chunk.last_arc);

        if snmp_conf.table_format {
            for (last_arc, value) in chunk.values {
                result.entry(last_arc).or_default().push(value);
            }
            if let Some(min_last_arc) = get_min_last_arc(last_oid_data.values()) {
                let oid = result
                    .range((Excluded(min_last_arc), Unbounded))
                    .next()
                    .map(|(oid, _)| oid.clone());
                let batch = match &oid {
                    None => std::mem::take(&mut result),
                    Some(oid) => {
                        let mut map = result.split_off(oid);
                        std::mem::swap(&mut map, &mut result);
                        map
                    }
                };
                if !batch.is_empty() {
                    let mut output: Vec<super::MonitorLog> = Vec::with_capacity(batch.len());
                    for (last_arc, values) in batch {
                        if row_is_incomplete(oid_count, max_unknown_oid, values.len()) {
                            total_dropped += 1;
                            continue;
                        }
                        total_rows += 1;
                        let attrs = vec![
                            (
                                ZStringOwned::from_static("snmp.id"),
                                last_arc.to_smolstr().into(),
                            ),
                            (ZStringOwned::from_static("snmp.data"), values.into()),
                        ];
                        let row = get_row(attrs);
                        output.push(super::MonitorLog {
                            row,
                            processors: item_conf.processors.clone(),
                            resource: resource.clone(),
                            kind: ItemKind::from(&item_conf.config),
                            stats: stats.clone(),
                        });
                    }
                    // The whole batch can be discarded by the filter above, and an
                    // empty BatchLog is work for the batcher with nothing in it.
                    if !output.is_empty() {
                        trace!(length = output.len(), "sending batch");
                        sender
                            .send_async(MonitorRecord::BatchLog(output))
                            .await
                            .map_err(|e| super::Error::dropped(e.to_string()))?;
                        trace!("batch sent");
                    }
                }
            };
        } else {
            total_rows += 1;
            let attrs = vec![
                (
                    ZStringOwned::from_static("snmp.oid"),
                    chunk.op.oid_chain.oid.to_smolstr().into(),
                ),
                (
                    ZStringOwned::from_static("snmp.data"),
                    chunk
                        .values
                        .into_iter()
                        .map(|(_, v)| v)
                        .collect::<Vec<_>>()
                        .into(),
                ),
            ];
            let row = get_row(attrs);
            sender
                .send_async(MonitorRecord::SingleLog(Box::new(super::MonitorLog {
                    row,
                    processors: item_conf.processors.clone(),
                    resource: resource.clone(),
                    kind: ItemKind::from(&item_conf.config),
                    stats: stats.clone(),
                })))
                .await
                .map_err(|e| super::Error::dropped(e.to_string()))?;
        }

        if !chunk.finished {
            walk_ops.push_back(chunk.op);
        }
    }
    let mut output: Vec<super::MonitorLog> = Vec::with_capacity(result.len());
    for (idx, values) in result {
        if row_is_incomplete(oid_count, max_unknown_oid, values.len()) {
            total_dropped += 1;
            continue;
        }
        total_rows += 1;
        let key = if snmp_conf.table_format {
            ZStringOwned::from_static("snmp.id")
        } else {
            ZStringOwned::from_static("snmp.oid")
        };
        let attrs = vec![
            (key, idx.to_smolstr().into()),
            (ZStringOwned::from_static("snmp.data"), values.into()),
        ];
        let row = get_row(attrs);
        output.push(super::MonitorLog {
            row,
            resource: resource.clone(),
            kind: ItemKind::from(&item_conf.config),
            processors: item_conf.processors.clone(),
            stats: stats.clone(),
        });
    }
    if total_dropped > 0 {
        stats.add_dropped(total_dropped);
    }
    debug!(elapsed = ?n_start.elapsed(), total_rows, total_dropped, "walk finished");
    let record = if output.is_empty() {
        MonitorRecord::Null
    } else {
        MonitorRecord::BatchLog(output)
    };
    Ok(super::Collected::new(record, total_rows))
}

/// Resolve `endpoint` and build a client for it on the shared UDP transport.
/// `walk` carries a walk item's own bulk settings; a plain GET has none.
async fn snmp_client(
    endpoint: &Endpoint,
    request_timeout: Duration,
    retries: u32,
    walk: Option<&SnmpWalk>,
) -> Result<Client, super::Error> {
    let ip_dest = lookup_host(match &endpoint.config {
        EndpointConfig::SnmpV1(ep_conf) => ep_conf.address.as_str(),
        EndpointConfig::SnmpV2C(ep_conf) => ep_conf.address.as_str(),
        EndpointConfig::SnmpV3(ep_conf) => ep_conf.address.as_str(),
        _ => {
            return Err(super::Error::config(format!(
                "not an snmp endpoint config: {:?}",
                discriminant(&endpoint.config)
            )));
        }
    })
    .await
    .context("failed to resolve host")
    .map_err(|e| super::Error::unreachable(e.to_string()))?
    .next()
    .ok_or_else(|| super::Error::unreachable("failed to resolve host".to_string()))?;
    let client;
    let mut walk_options = WalkOptions::default();
    match &endpoint.config {
        EndpointConfig::SnmpV1(ep_conf) => {
            client = Client::builder(ip_dest, Auth::v1(&ep_conf.community));
        }
        EndpointConfig::SnmpV2C(ep_conf) => {
            client = Client::builder(ip_dest, Auth::v2c(&ep_conf.community));
            walk_options.method = walk_method(ep_conf.disable_bulk);
            walk_options.max_repetitions = ep_conf.max_bulk_repetitions;
        }
        EndpointConfig::SnmpV3(ep_conf) => {
            // async-snmp refuses DES/3DES privacy without a salt state;
            // unused by the other protocols.
            client = Client::builder(ip_dest, &ep_conf.security_model)
                .engine_cache(ENGINE_CACHE.clone())
                .des_salt_state(crate::common::snmp::des_salt_state());
            walk_options.method = walk_method(ep_conf.disable_bulk);
            walk_options.max_repetitions = ep_conf.max_bulk_repetitions;
        }
        _ => unreachable!("already handled above"),
    };
    if let Some(walk) = walk {
        // The item's `disable_bulk` overrides the endpoint's; unset, it inherits.
        if walk.disable_bulk.is_some() {
            walk_options.method = walk_method(walk.disable_bulk);
        }
        if walk.max_results > 0 {
            walk_options.result_limit = Some(walk.max_results);
        }
    }
    let retry =
        Retry::fixed(retries, Duration::ZERO).map_err(|e| super::Error::config(e.to_string()))?;
    client
        .walk_options(walk_options)
        .retry(retry)
        .request_timeout(request_timeout)
        .build_with(get_udp_transport(ip_dest).await)
        .await
        .map_err(|e| map_snmp_err(&e))
}

pub async fn process_snmp_walk(
    item: &Item,
    endpoint: &Endpoint,
    host: &Host,
    stats: &Arc<Stats>,
    sender: &flume::Sender<MonitorRecord>,
) -> Result<super::Collected, super::Error> {
    let ItemConfig::SnmpWalk(item_conf) = &item.config else {
        return Err(super::Error::config(format!(
            "not an snmp walk config: {:?}",
            discriminant(&item.config),
        )));
    };
    let client = snmp_client(
        endpoint,
        item_conf.request_timeout,
        item_conf.request_retries,
        Some(item_conf),
    )
    .await?;
    snmp_walk(client, item, endpoint, host, item_conf, stats, sender).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::item::FailureKind;
    use crate::config::snmp::OidPositions;

    /// ifXTable entry, standing in for a chained oid
    const BASE: &str = "1.3.6.1.2.1.31.1.1.1.18";

    fn oid(s: &str) -> Oid {
        Oid::parse(s).expect("valid oid")
    }

    fn pos(start: usize, end: usize) -> OidPositions {
        OidPositions { start, end }
    }

    fn child(strategy: ChainStrategy, last_arc: &str, value: Value) -> Option<Oid> {
        build_child_oid(&oid(BASE), &strategy, &oid(last_arc), &value)
    }

    #[test]
    fn value_strategy_appends_an_integer_arc() {
        assert_eq!(
            child(ChainStrategy::Value, "7", Value::Integer(42)),
            Some(oid("1.3.6.1.2.1.31.1.1.1.18.42"))
        );
    }

    #[test]
    fn value_strategy_appends_ip_address_octets() {
        assert_eq!(
            child(ChainStrategy::Value, "7", Value::IpAddress([10, 0, 0, 1])),
            Some(oid("1.3.6.1.2.1.31.1.1.1.18.10.0.0.1"))
        );
    }

    #[test]
    fn value_strategy_skips_values_that_are_not_arcs() {
        assert_eq!(child(ChainStrategy::Value, "7", Value::Counter32(9)), None);
        assert_eq!(child(ChainStrategy::Value, "7", Value::Null), None);
    }

    #[test]
    fn oid_position_appends_the_selected_arcs_of_the_walked_suffix() {
        assert_eq!(
            child(
                ChainStrategy::OidPosition(pos(0, 1)),
                "10.20.30.40",
                Value::Null
            ),
            Some(oid("1.3.6.1.2.1.31.1.1.1.18.10.20"))
        );
    }

    #[test]
    fn oid_position_reversed_appends_the_trailing_arcs() {
        assert_eq!(
            child(
                ChainStrategy::OidPositionReversed(pos(0, 1)),
                "10.20.30.40",
                Value::Null
            ),
            Some(oid("1.3.6.1.2.1.31.1.1.1.18.30.40"))
        );
    }

    #[test]
    fn oid_position_strategies_ignore_the_varbind_value() {
        let strategy = ChainStrategy::OidPosition(pos(0, 0));
        let from_null = child(strategy, "10.20", Value::Null);
        let from_integer = child(strategy, "10.20", Value::Integer(99));
        assert_eq!(from_null, Some(oid("1.3.6.1.2.1.31.1.1.1.18.10")));
        assert_eq!(from_null, from_integer);
    }

    #[test]
    fn an_unset_threshold_keeps_every_row() {
        assert!(!row_is_incomplete(5, None, 0));
        assert!(!row_is_incomplete(5, None, 5));
    }

    #[test]
    fn the_threshold_is_a_budget_the_row_is_allowed_to_spend() {
        // 5 configured oids, 2 misses allowed: 3 values is exactly on budget.
        assert!(!row_is_incomplete(5, Some(2), 3));
        assert!(row_is_incomplete(5, Some(2), 2));
    }

    #[test]
    fn a_zero_threshold_demands_every_oid() {
        assert!(!row_is_incomplete(5, Some(0), 5));
        assert!(row_is_incomplete(5, Some(0), 4));
    }

    #[test]
    fn more_values_than_configured_oids_is_not_a_miss() {
        // Can't happen — each oid contributes at most one value per row — but the
        // shortfall must saturate rather than wrap into a huge miss count.
        assert!(!row_is_incomplete(5, Some(0), 6));
    }

    fn snmp_entry(oid_str: &str, use_mib: bool) -> (cache::item::Entry, Snmp) {
        let config = Snmp {
            oid: oid(oid_str),
            use_mib,
        };
        let item = Item::new(
            Uuid::now_v7(),
            "test".into(),
            None,
            Uuid::now_v7(),
            crate::config::item::Schedule::Never,
            None,
            Duration::from_secs(1),
            vec![],
            ItemConfig::Snmp(config.clone()),
        );
        (cache::item::Entry::new(item), config)
    }

    #[test]
    fn collect_rows_sorts_answers_into_rows_and_per_item_failures() {
        let host = Host {
            id: Uuid::now_v7(),
            name: "host".into(),
            timezone: None,
        };
        let endpoint = Endpoint::new(
            Uuid::now_v7(),
            "agent".into(),
            host.id,
            EndpointConfig::SnmpV1(crate::config::endpoint::SnmpV1 {
                address: "127.0.0.1:161".parse().unwrap(),
                community: "public".into(),
            }),
        );
        let answered = snmp_entry("1.3.6.1.2.1.1.5.0", false);
        let unbound = snmp_entry("1.3.6.1.2.1.1.6.0", false);
        let excepted = snmp_entry("1.3.6.1.2.1.1.7.0", false);
        let no_such_name = snmp_entry("1.3.6.1.2.1.1.8.0", false);
        let items: Vec<_> = [&answered, &unbound, &excepted, &no_such_name]
            .into_iter()
            .map(|(entry, config)| (entry, config)) // &(A, B) -> (&A, &B)
            .collect();
        let values = FxHashMap::from_iter([
            (answered.1.oid.clone(), Value::Integer(7)),
            (excepted.1.oid.clone(), Value::NoSuchInstance),
        ]);
        let unsupported = FxHashSet::from_iter([no_such_name.1.oid.clone()]);

        let (rows, failed) = collect_rows(
            &items,
            &endpoint,
            &host,
            "127.0.0.1:161".parse().unwrap(),
            &values,
            &unsupported,
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].resource.item_id, answered.0.item.id);
        let payload::ValueOwned::Array(data) = &rows[0].row.attrs[1].1 else {
            panic!("snmp.data is a list");
        };
        let [payload::ValueOwned::KvList(fields)] = data.as_slice() else {
            panic!("snmp.data holds one value");
        };
        let keys: Vec<&str> = fields.iter().map(|(k, _)| k.as_ref()).collect();
        assert_eq!(keys, ["oid", "value"], "no MIB fields without use_mib");

        let kind = |entry: &cache::item::Entry| failed[&entry.item.id].kind;
        assert_eq!(failed.len(), 3);
        assert_eq!(kind(&unbound.0), FailureKind::Malformed);
        assert_eq!(kind(&excepted.0), FailureKind::NotSupported);
        assert_eq!(kind(&no_such_name.0), FailureKind::NotSupported);
    }

    #[test]
    fn a_suffix_too_short_for_the_positions_skips_the_chain() {
        // a bogus child oid would be silently walked, so this must be None
        assert_eq!(
            child(ChainStrategy::OidPosition(pos(0, 3)), "7", Value::Null),
            None
        );
        assert_eq!(
            child(
                ChainStrategy::OidPositionReversed(pos(0, 3)),
                "7",
                Value::Null
            ),
            None
        );
    }

    fn v3_endpoint(privacy: &str) -> Endpoint {
        serde_yaml_ng::from_str(&format!(
            "id: 019b77b3-df79-70a2-ab01-f43b2cf723a4
name: ep
host_id: 019b77b3-df79-70a2-ab01-f43b2cf723a5
type: snmp_v3
config:
  address: 127.0.0.1:1
  security_model:
    type: usm
    config:
      username: user
      authentication: {{protocol: sha1, passphrase: authpass123}}
      privacy: {{protocol: {privacy}, passphrase: privpass123}}
"
        ))
        .unwrap()
    }

    /// async-snmp 0.18 refuses to build a client with DES or 3DES privacy
    /// unless it is given a salt state, so every poll of such an endpoint
    /// failed before it was sent.
    #[test]
    fn a_des_authpriv_endpoint_builds_a_client() {
        TEST_RUNTIME.block_on(async {
            for privacy in ["des", "des3", "aes128"] {
                snmp_client(&v3_endpoint(privacy), Duration::from_secs(1), 0, None)
                    .await
                    .unwrap_or_else(|e| panic!("{privacy}: {e}"));
            }
        });
    }
}
