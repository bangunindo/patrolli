// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod fields;
mod sflow;

use crate::chan::SenderChannel;
use crate::common::{ByteCounters, UdpListenerWorker};
use crate::config::listener::ListenerKind;
use crate::listener::InternalMetrics;
use crate::payload::{LogRow, Scope, Value, ZString};
use crate::{cache, config, payload};
use ahash::{HashMap, HashSet};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fields::{
    ipfix_cisco_field_map, ipfix_iana_field_map, ipfix_nat_field_map, ipfix_netscaler_field_map,
    ipfix_reverse_field_map, ipfix_vmware_field_map, ipfix_yaf_field_map, prototype_field_map,
    v9_field_map,
};
use netflow_parser::variable_versions::field_value::{DataNumber, DurationValue, FieldValue};
use netflow_parser::variable_versions::ipfix::lookup::{IANAIPFixField, IPFixField};
use netflow_parser::variable_versions::ipfix::{FlowSetBody as IpfixFlowSetBody, IPFixFlowRecord};
use netflow_parser::variable_versions::v9::{
    FlowSetBody as V9FlowSetBody, OptionsDataFields, ScopeDataField, V9FlowRecord,
};
use netflow_parser::{AutoScopedParser, NetflowPacket, V9Field};
use parking_lot::Mutex as ParkingMutex;
use rustc_hash::FxHasher;
use smol_str::{SmolStr, ToSmolStr, format_smolstr};
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;
use tokio::task::JoinSet;
use tokio::time::{Instant, interval};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, Span, debug, error, info, trace, warn};
use uuid::Uuid;

type UdpReceiver = flume::Receiver<Vec<(bytes::Bytes, SocketAddr)>>;

pub struct Netflow {
    conf: config::listener::Listener,
    channel: SenderChannel<&'static str, payload::ProcessorPayload>,
    forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
    conf_cache: cache::Cache,
    node_id: Option<Uuid>,

    internal_metrics: InternalMetrics,
    /// The readers' bytes, kept across restarts of the readers.
    udp_metrics: Arc<ByteCounters>,
    state: TokioMutex<State>,
    /// Per-source-IP shards. Template and sampling state is pinned to a shard
    /// by [`shard_index`], so a source's data flowsets always meet the
    /// templates it sent earlier.
    shards: Arc<[ParkingMutex<Shard>]>,
}

/// One partition of the per-source state, keyed by exporter IP.
struct Shard {
    parser: AutoScopedParser,
    /// Sampling rates each templated source announced in options records. Lives as long as the source's templates: [`Shard::prune_idle`].
    sampling: HashMap<SamplingRateKey, Arc<SamplerRates>>,
}

impl Shard {
    fn new(parser: AutoScopedParser) -> Self {
        Self {
            parser,
            sampling: HashMap::default(),
        }
    }

    /// Drops the sources idle for `idle_after`, then the sampling rates of
    /// every source the parser no longer holds: the ones just pruned and the
    /// ones its LRU evicted since the last sweep. Returns both counts.
    fn prune_idle(&mut self, idle_after: Duration) -> (usize, usize) {
        let pruned = self.parser.prune_idle_sources(idle_after);
        if self.sampling.is_empty() {
            return (pruned, 0);
        }
        let v9: HashSet<(IpAddr, u32)> = self
            .parser
            .v9_info()
            .iter()
            .map(|(k, _)| (k.addr.ip(), k.source_id))
            .collect();
        let ipfix: HashSet<(IpAddr, u32)> = self
            .parser
            .ipfix_info()
            .iter()
            .map(|(k, _)| (k.addr.ip(), k.observation_domain_id))
            .collect();
        let before = self.sampling.len();
        self.sampling.retain(|key, _| {
            let live = match key.version {
                TemplatedVersion::V9 => &v9,
                TemplatedVersion::IPFix => &ipfix,
            };
            live.contains(&(key.addr, key.engine_id))
        });
        (pruned, before - self.sampling.len())
    }
}

/// Counts for one [`Netflow::handle_packets`] call, flushed into
/// [`InternalMetrics`] once at its end so the per-record path touches no cache
/// line shared with the other workers.
#[derive(Default)]
struct Tally {
    records: u64,
    records_dropped: u64,
    items_succeeded: u64,
    items_dropped: u64,
}

impl Tally {
    fn flush(self, m: &InternalMetrics) {
        for (counter, n) in [
            (&m.total_records, self.records),
            (&m.total_records_dropped, self.records_dropped),
            (&m.total_items_succeeded, self.items_succeeded),
            (&m.total_items_dropped, self.items_dropped),
        ] {
            if n > 0 {
                counter.fetch_add(n, Ordering::Relaxed);
            }
        }
    }
}

/// What the transport is doing. Workers start it; only `stop` ends it.
enum State {
    Idle,
    Running(Running),
    /// Stopped by the listener's close; never started again.
    Closed,
}

/// Started reader threads and the idle-source pruner. The readers hold the
/// only senders of `rx`'s channel, so it disconnects once they have stopped.
/// Dropping it stops them.
struct Running {
    rx: UdpReceiver,
    udp: UdpListenerWorker,
    intake: CancellationToken,
    /// [`prune_loop`].
    pruner: JoinSet<()>,
}

impl Running {
    async fn shutdown(mut self) {
        self.intake.cancel();
        // Only the workers' receivers are left, so with none running (scaled
        // to zero) the readers' last sends fail at once instead of waiting
        // out the drain timeout.
        drop(self.rx);
        self.udp.close().await;
        while let Some(res) = self.pruner.join_next().await {
            if let Err(e) = res {
                error!(error = %e, "pruner task failed");
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SamplingRateKey {
    version: TemplatedVersion,
    addr: IpAddr,
    engine_id: u32,
}

impl SamplingRateKey {
    fn new(version: TemplatedVersion, addr: SocketAddr, engine_id: u32) -> Self {
        Self {
            version,
            addr: addr.ip(),
            engine_id,
        }
    }

    /// The key a templated packet's sampling rate is kept under; `None` for
    /// the versions that carry their rate in the header.
    fn of(addr: SocketAddr, packet: &NetflowPacket) -> Option<Self> {
        match packet {
            NetflowPacket::V9(data) => {
                Some(Self::new(TemplatedVersion::V9, addr, data.header.source_id))
            }
            NetflowPacket::IPFix(data) => Some(Self::new(
                TemplatedVersion::IPFix,
                addr,
                data.header.observation_domain_id,
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TemplatedVersion {
    V9,
    IPFix,
}

/// The sampling rates one exporter (a [`SamplingRateKey`]) announced in
/// options records, by sampler / selector ID. [`GENERIC_SAMPLER`] holds the
/// rate of an options record that names no sampler.
type SamplerRates = HashMap<u64, u32>;

/// The sampler an options record without a sampler ID describes, and the one
/// a data record falls back to when its own is unknown or absent.
const GENERIC_SAMPLER: u64 = 0;

/// The rate the exporter announced for sampler `id`, else its generic rate.
/// What earlier flowsets of this batch announced (`detected`) is newer than
/// the shard's copy (`cached`).
fn lookup_rate(
    detected: Option<&SamplerRates>,
    cached: Option<&SamplerRates>,
    id: u64,
) -> Option<u32> {
    let get = |id| {
        detected
            .and_then(|r| r.get(&id))
            .or_else(|| cached.and_then(|r| r.get(&id)))
            .copied()
    };
    get(id).or_else(|| {
        (id != GENERIC_SAMPLER)
            .then(|| get(GENERIC_SAMPLER))
            .flatten()
    })
}

/// The sampling fields of one data or options record (RFC 7270 and the
/// PSAMP elements of RFC 5476 that replace them).
#[derive(Default)]
struct SamplingFields {
    /// v9 `FLOW_SAMPLER_ID`, IPFIX `samplerId` / `selectorId`.
    sampler_id: Option<u64>,
    /// `SAMPLING_INTERVAL` (34).
    interval: Option<u64>,
    /// `FLOW_SAMPLER_RANDOM_INTERVAL` (50).
    random_interval: Option<u64>,
    /// `samplingPacketInterval` (305).
    packet_interval: Option<u64>,
    /// `samplingPacketSpace` (306).
    packet_space: Option<u64>,
    /// `samplingSize` (309).
    size: Option<u64>,
    /// `samplingPopulation` (310).
    population: Option<u64>,
    /// `samplingProbability` (311).
    probability: Option<f64>,
}

impl SamplingFields {
    fn observe_v9(&mut self, field: V9Field, value: &FieldValue) {
        let FieldValue::DataNumber(d) = value else {
            return;
        };
        let slot = match field {
            V9Field::FlowSamplerId => &mut self.sampler_id,
            V9Field::FlowSamplerRandomInterval => &mut self.random_interval,
            V9Field::SamplingInterval => &mut self.interval,
            _ => return,
        };
        *slot = d.as_u64();
    }

    fn observe_ipfix(&mut self, field: IPFixField, value: &FieldValue) {
        let IPFixField::IANA(iana) = field else {
            return;
        };
        if iana == IANAIPFixField::SamplingProbability {
            if let FieldValue::Float64(p) = value {
                self.probability = Some(*p);
            }
            return;
        }
        let FieldValue::DataNumber(d) = value else {
            return;
        };
        let slot = match iana {
            IANAIPFixField::SamplerId | IANAIPFixField::SelectorId => &mut self.sampler_id,
            IANAIPFixField::SamplerRandomInterval => &mut self.random_interval,
            IANAIPFixField::SamplingInterval => &mut self.interval,
            IANAIPFixField::SamplingPacketInterval => &mut self.packet_interval,
            IANAIPFixField::SamplingPacketSpace => &mut self.packet_space,
            IANAIPFixField::SamplingSize => &mut self.size,
            IANAIPFixField::SamplingPopulation => &mut self.population,
            _ => return,
        };
        *slot = d.as_u64();
    }

    /// The "1 in N" rate these fields describe:
    /// - random n-out-of-N: `population / size`;
    /// - systematic count-based: `interval` packets out of every
    ///   `interval + space`;
    /// - uniform probabilistic: `1 / probability`;
    /// - otherwise the first explicit interval present, 305 being read as
    ///   "1 in N" when it comes without a space, as exporters send it.
    fn rate(&self) -> Option<u32> {
        let rate = if let (Some(population), Some(size)) = (self.population, self.size)
            && size > 0
        {
            population / size
        } else if let (Some(interval), Some(space)) = (self.packet_interval, self.packet_space)
            && interval > 0
        {
            interval.saturating_add(space) / interval
        } else if let Some(p) = self.probability
            && p > 0.0
            && p <= 1.0
        {
            (1.0 / p).round() as u64
        } else {
            self.packet_interval
                .or(self.random_interval)
                .or(self.interval)?
        };
        u32::try_from(rate).ok().filter(|&rate| rate > 0)
    }
}

/// What every row of one templated packet shares.
struct PacketCtx<'c> {
    addr: SocketAddr,
    capture_time: Option<DateTime<Utc>>,
    ty: &'static str,
    sequence_number: Option<u32>,
    engine_id: Option<u32>,
    uptime: Option<f64>,
    /// The exporter the packet came from, which its rates are kept under.
    key: SamplingRateKey,
    /// The shard's rates for that exporter.
    cached: Option<&'c SamplerRates>,
}

impl PacketCtx<'_> {
    /// The rate a data record was sampled at: the one it carries, else its
    /// sampler's, else the exporter's generic one.
    fn data_rate(
        &self,
        detected: &HashMap<SamplingRateKey, SamplerRates>,
        sampling: &SamplingFields,
    ) -> Option<u32> {
        sampling.rate().or_else(|| {
            lookup_rate(
                detected.get(&self.key),
                self.cached,
                sampling.sampler_id.unwrap_or(GENERIC_SAMPLER),
            )
        })
    }

    /// Notes the rate an options record announces, for the flowsets after it
    /// and, through the write-back, later batches. Only a change is noted, so
    /// the write-back stays empty while rates hold.
    fn announce(
        &self,
        detected: &mut HashMap<SamplingRateKey, SamplerRates>,
        sampling: &SamplingFields,
    ) {
        let Some(rate) = sampling.rate() else {
            return;
        };
        let id = sampling.sampler_id.unwrap_or(GENERIC_SAMPLER);
        let known = detected
            .get(&self.key)
            .and_then(|r| r.get(&id))
            .or_else(|| self.cached.and_then(|r| r.get(&id)));
        if known != Some(&rate) {
            detected.entry(self.key).or_default().insert(id, rate);
        }
    }

    fn emit<'a>(
        &self,
        attrs: &mut Vec<(ZString<'a>, Value<'a>)>,
        flow_set_id: &'a str,
        sampling_rate: Option<u32>,
        batcher: &mut payload::LogsBuilder,
    ) {
        push_header(
            attrs,
            self.ty,
            self.sequence_number,
            sampling_rate,
            self.engine_id,
            Some(flow_set_id),
            self.uptime,
        );
        batcher.push_row(Netflow::flow_row(self.addr, self.capture_time, attrs));
    }
}

/// Push the protocol-level NetFlow/sFlow header fields onto a row's attribute
/// list under the `nf.` namespace. Shared with the sFlow path so both
/// produce identically-shaped rows. `capture_time` is intentionally not here:
/// it maps onto the log row's `observed_time` instead.
pub(super) fn push_header<'a>(
    attrs: &mut Vec<(ZString<'a>, Value<'a>)>,
    ty: &'static str,
    sequence_number: Option<u32>,
    sampling_rate: Option<u32>,
    engine_id: Option<u32>,
    flow_set_id: Option<&'a str>,
    uptime: Option<f64>,
) {
    attrs.push((
        ZString::from_static("nf.type"),
        Value::String(ZString::from_ref(ty)),
    ));
    if let Some(v) = sequence_number {
        attrs.push((
            ZString::from_static("nf.sequence_number"),
            Value::Int64(v as i64),
        ));
    }
    if let Some(v) = sampling_rate {
        attrs.push((
            ZString::from_static("nf.sampling_rate"),
            Value::Int64(v as i64),
        ));
    }
    if let Some(v) = engine_id {
        attrs.push((ZString::from_static("nf.engine_id"), Value::Int64(v as i64)));
    }
    if let Some(v) = flow_set_id {
        attrs.push((
            ZString::from_static("nf.flow_set_id"),
            Value::String(ZString::from_ref(v)),
        ));
    }
    if let Some(v) = uptime {
        attrs.push((ZString::from_static("nf.uptime"), Value::Double(v)));
    }
}

/// `bytes` as a big-endian unsigned integer, when it has 1 to 8 of them.
fn be_uint(bytes: &[u8]) -> Option<u64> {
    if !(1..=8).contains(&bytes.len()) {
        return None;
    }
    let mut be = [0u8; 8];
    be[8 - bytes.len()..].copy_from_slice(bytes);
    Some(u64::from_be_bytes(be))
}

/// Pick the parser shard owning `addr`. Keyed on the IP alone so every datagram
/// from one exporter lands on the same shard — its templates live there — and
/// stable across restarts because `FxHasher` is fixed-seed.
fn shard_index(addr: SocketAddr, shards: usize) -> usize {
    let mut hasher = FxHasher::default();
    addr.ip().hash(&mut hasher);
    hasher.finish() as usize % shards
}

impl Netflow {
    pub fn new(
        conf: config::listener::Listener,
        channel: SenderChannel<&'static str, payload::ProcessorPayload>,
        forwarder: Option<SenderChannel<&'static str, payload::ForwarderPayload>>,
        conf_cache: cache::Cache,
        node_id: Option<Uuid>,
    ) -> Self {
        let max_sources = match &conf.config {
            config::listener::Config::Netflow(conf) => conf.max_sources,
            _ => panic!("not netflow config"),
        };
        let shards = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        // `max_sources` is per shard, so the process-wide ceiling is this times
        // `shards`. `validate` already rules out zero; the clamp keeps
        // `with_max_sources` infallible even if a caller skipped it.
        let per_shard = max_sources.max(1);
        let shards = (0..shards)
            .map(|_| {
                // `AutoScopedParser::new()` eagerly allocates three LruCaches at
                // the library default of 10k entries each. Stepping through 1
                // first shrinks them while still empty, so they grow lazily --
                // going straight to `per_shard` would hit `resize`'s
                // `cap == self.cap` early return on the default and keep the
                // full preallocation on every shard.
                let parser = AutoScopedParser::new()
                    .with_max_sources(1)
                    .expect("1 is non-zero")
                    .with_max_sources(per_shard)
                    .expect("per-shard max_sources is non-zero");
                ParkingMutex::new(Shard::new(parser))
            })
            .collect::<Arc<[_]>>();
        Self {
            conf,
            channel,
            forwarder,
            conf_cache,
            node_id,
            internal_metrics: InternalMetrics::default(),
            udp_metrics: Arc::default(),
            state: TokioMutex::new(State::Idle),
            shards,
        }
    }
    fn conf(&self) -> &config::listener::Netflow {
        match &self.conf.config {
            config::listener::Config::Netflow(conf) => conf,
            _ => panic!("not netflow config"),
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

    fn encode_data_number(value: &DataNumber) -> Value<'static> {
        match value {
            DataNumber::U8(value) => Value::Int64(*value as i64),
            DataNumber::I8(value) => Value::Int64(*value as i64),
            DataNumber::U16(value) => Value::Int64(*value as i64),
            DataNumber::I16(value) => Value::Int64(*value as i64),
            DataNumber::U24(value) => Value::Int64(*value as i64),
            DataNumber::I24(value) => Value::Int64(*value as i64),
            DataNumber::U32(value) => Value::Int64(*value as i64),
            DataNumber::I32(value) => Value::Int64(*value as i64),
            DataNumber::U64(value) => {
                if *value > i64::MAX as u64 {
                    Value::String(ZString::from_owned(value.to_string()))
                } else {
                    Value::Int64(*value as i64)
                }
            }
            DataNumber::I64(value) => Value::Int64(*value),
            DataNumber::U128(value) => {
                if *value > i64::MAX as u128 {
                    Value::String(ZString::from_owned(value.to_string()))
                } else {
                    Value::Int64(*value as i64)
                }
            }
            DataNumber::I128(value) => {
                if *value > i64::MAX as i128 || *value < i64::MIN as i128 {
                    Value::String(ZString::from_owned(value.to_string()))
                } else {
                    Value::Int64(*value as i64)
                }
            }
            DataNumber::Vec(value) => {
                let value = num_bigint::BigInt::from_signed_bytes_be(value);
                if value > num_bigint::BigInt::from(i64::MAX)
                    || value < num_bigint::BigInt::from(i64::MIN)
                {
                    Value::String(ZString::from_owned(value.to_string()))
                } else {
                    Value::Int64(value.try_into().unwrap())
                }
            }
        }
    }

    fn extract_v9_field_str(field: V9Field) -> ZString<'static> {
        v9_field_map(&field)
            .map(ZString::from_static)
            .unwrap_or_else(|num| ZString::from_smol(format_smolstr!("nf.data.v9_{num}")))
    }

    // TODO: add ways to configure enterprise field names
    fn extract_ipfix_field_str(field: IPFixField) -> ZString<'static> {
        match field {
            IPFixField::IANA(field) => ipfix_iana_field_map(&field)
                .map(ZString::from_static)
                .unwrap_or_else(|num| ZString::from_smol(format_smolstr!("nf.data.iana_{}", num))),
            IPFixField::Cisco(field) => ipfix_cisco_field_map(&field)
                .map(ZString::from_static)
                .unwrap_or_else(|num| ZString::from_smol(format_smolstr!("nf.data.cisco_{}", num))),
            IPFixField::Netscaler(field) => ipfix_netscaler_field_map(&field)
                .map(ZString::from_static)
                .unwrap_or_else(|num| {
                    ZString::from_smol(format_smolstr!("nf.data.netscaler_{}", num))
                }),
            IPFixField::Nat(field) => ipfix_nat_field_map(&field)
                .map(ZString::from_static)
                .unwrap_or_else(|num| ZString::from_smol(format_smolstr!("nf.data.nat_{}", num))),
            IPFixField::Yaf(field) => ipfix_yaf_field_map(&field)
                .map(ZString::from_static)
                .unwrap_or_else(|num| ZString::from_smol(format_smolstr!("nf.data.yaf_{}", num))),
            IPFixField::VMWare(field) => ipfix_vmware_field_map(&field)
                .map(ZString::from_static)
                .unwrap_or_else(|num| {
                    ZString::from_smol(format_smolstr!("nf.data.vmware_{}", num))
                }),
            IPFixField::ReverseInformationElement(field) => ipfix_reverse_field_map(&field)
                .map(ZString::from_static)
                .unwrap_or_else(|num| ZString::from_smol(format_smolstr!("nf.data.rie_{}", num))),
            IPFixField::Enterprise {
                enterprise_number,
                field_number,
            } => ZString::from_smol(format_smolstr!(
                "nf.data.ent_{}_{}",
                enterprise_number,
                field_number
            )),
        }
    }

    /// Convert a single parsed NetFlow field value into a payload `Value`,
    /// borrowing from `value` (strings, byte vectors) rather than copying where
    /// the parser already owns the bytes. Returns `None` for field variants we
    /// don't model, so they're skipped rather than emitted as null attributes.
    fn field_value_to_value(value: &FieldValue) -> Option<Value<'_>> {
        Some(match value {
            FieldValue::ApplicationId(value) => {
                let selector = match &value.selector_id {
                    Some(selector_id) => Self::encode_data_number(selector_id),
                    None => Value::Null,
                };
                Value::KvList(vec![
                    (
                        ZString::from_ref("classification_engine_id"),
                        Value::Int64(value.classification_engine_id as i64),
                    ),
                    (ZString::from_ref("selector_id"), selector),
                ])
            }
            FieldValue::String(value) => Value::String(ZString::from_ref(value.value.as_str())),
            FieldValue::DataNumber(value) => Self::encode_data_number(value),
            FieldValue::Float64(value) => Value::Double(*value),
            FieldValue::Duration(value) => {
                let dur = value.as_duration();
                let num = match value {
                    DurationValue::Seconds { .. } => {
                        i64::try_from(dur.as_secs()).unwrap_or(i64::MAX)
                    }
                    DurationValue::Millis { .. } => {
                        i64::try_from(dur.as_millis()).unwrap_or(i64::MAX)
                    }
                    DurationValue::MicrosNtp { .. } => {
                        i64::try_from(dur.as_micros()).unwrap_or(i64::MAX)
                    }
                    // nanos will overflow in year 2262 if this is unix timestamp
                    DurationValue::NanosNtp { .. } => {
                        i64::try_from(dur.as_nanos()).unwrap_or(i64::MAX)
                    }
                };
                Value::Int64(num)
            }
            FieldValue::Ip4Addr(value) => Value::String(ZString::from_smol(value.to_smolstr())),
            FieldValue::Ip6Addr(value) => Value::String(ZString::from_smol(value.to_smolstr())),
            FieldValue::MacAddr(value) => Value::String(ZString::from_smol(format_smolstr!(
                "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                value[0],
                value[1],
                value[2],
                value[3],
                value[4],
                value[5]
            ))),
            FieldValue::Vec(value) => Value::Bytes(Cow::Borrowed(value.as_slice())),
            FieldValue::ProtocolType(value) => match prototype_field_map(value) {
                Some(value) => Value::String(ZString::from_ref(value)),
                None => {
                    let value: u8 = (*value).into();
                    Value::Int64(value as i64)
                }
            },
            FieldValue::ForwardingStatus(value) => {
                Value::String(ZString::from_smol(value.to_smolstr()))
            }
            FieldValue::FragmentFlags(value) => {
                let mut flags: Vec<Value> = Vec::with_capacity(3);
                if value.dont_fragment {
                    flags.push(Value::String(ZString::from_ref("DF")));
                }
                if value.more_fragments {
                    flags.push(Value::String(ZString::from_ref("MF")));
                }
                if value.reserved {
                    flags.push(Value::String(ZString::from_ref("R")));
                }
                Value::Array(flags)
            }
            FieldValue::TcpControlBits(value, _pad) => {
                let mut flags: Vec<Value> = Vec::with_capacity(9);
                if value.ns {
                    flags.push(Value::String(ZString::from_ref("NS")));
                }
                if value.cwr {
                    flags.push(Value::String(ZString::from_ref("CWR")));
                }
                if value.ece {
                    flags.push(Value::String(ZString::from_ref("ECE")));
                }
                if value.urg {
                    flags.push(Value::String(ZString::from_ref("URG")));
                }
                if value.ack {
                    flags.push(Value::String(ZString::from_ref("ACK")));
                }
                if value.psh {
                    flags.push(Value::String(ZString::from_ref("PSH")));
                }
                if value.rst {
                    flags.push(Value::String(ZString::from_ref("RST")));
                }
                if value.syn {
                    flags.push(Value::String(ZString::from_ref("SYN")));
                }
                if value.fin {
                    flags.push(Value::String(ZString::from_ref("FIN")));
                }
                Value::Array(flags)
            }
            FieldValue::Ipv6ExtensionHeaders(value) => {
                let mut flags: Vec<Value> = Vec::with_capacity(6);
                if value.destination_options {
                    flags.push(Value::String(ZString::from_ref("DST")));
                }
                if value.fragment {
                    flags.push(Value::String(ZString::from_ref("FRAG")));
                }
                if value.hop_by_hop {
                    flags.push(Value::String(ZString::from_ref("HBH")));
                }
                if value.routing {
                    flags.push(Value::String(ZString::from_ref("RT")));
                }
                if value.authentication {
                    flags.push(Value::String(ZString::from_ref("AH")));
                }
                if value.esp {
                    flags.push(Value::String(ZString::from_ref("ESP")));
                }
                Value::Array(flags)
            }
            FieldValue::Ipv4Options(value) => {
                let mut flags: Vec<Value> = Vec::with_capacity(23);
                if value.rr {
                    flags.push(Value::String(ZString::from_ref("RR")));
                }
                if value.cipso {
                    flags.push(Value::String(ZString::from_ref("CIPSO")));
                }
                if value.e_sec {
                    flags.push(Value::String(ZString::from_ref("E-SEC")));
                }
                if value.ts {
                    flags.push(Value::String(ZString::from_ref("TS")));
                }
                if value.lsr {
                    flags.push(Value::String(ZString::from_ref("LSR")));
                }
                if value.sec {
                    flags.push(Value::String(ZString::from_ref("SEC")));
                }
                if value.nop {
                    flags.push(Value::String(ZString::from_ref("NOP")));
                }
                if value.eool {
                    flags.push(Value::String(ZString::from_ref("EOOL")));
                }
                if value.encode {
                    flags.push(Value::String(ZString::from_ref("ENCODE")));
                }
                if value.visa {
                    flags.push(Value::String(ZString::from_ref("VISA")));
                }
                if value.finn {
                    flags.push(Value::String(ZString::from_ref("FINN")));
                }
                if value.mtur {
                    flags.push(Value::String(ZString::from_ref("MTUR")));
                }
                if value.mtup {
                    flags.push(Value::String(ZString::from_ref("MTUP")));
                }
                if value.zsu {
                    flags.push(Value::String(ZString::from_ref("ZSU")));
                }
                if value.ssr {
                    flags.push(Value::String(ZString::from_ref("SSR")));
                }
                if value.sid {
                    flags.push(Value::String(ZString::from_ref("SID")));
                }
                if value.dps {
                    flags.push(Value::String(ZString::from_ref("DPS")));
                }
                if value.nsapa {
                    flags.push(Value::String(ZString::from_ref("NSAPA")));
                }
                if value.sdb {
                    flags.push(Value::String(ZString::from_ref("SDB")));
                }
                if value.rtralt {
                    flags.push(Value::String(ZString::from_ref("RTRALT")));
                }
                if value.tr {
                    flags.push(Value::String(ZString::from_ref("TR")));
                }
                if value.eip {
                    flags.push(Value::String(ZString::from_ref("EIP")));
                }
                if value.imitd {
                    flags.push(Value::String(ZString::from_ref("IMITD")));
                }
                Value::Array(flags)
            }
            FieldValue::TcpOptions(value) => Value::String(ZString::from_smol(value.to_smolstr())),
            FieldValue::IsMulticast(value) => Value::String(ZString::from_smol(value.to_smolstr())),
            FieldValue::MplsLabelExp(value) => Value::Int64(value.0 as i64),
            FieldValue::FlowEndReason(value) => {
                Value::String(ZString::from_smol(value.to_smolstr()))
            }
            FieldValue::NatEvent(value) => Value::String(ZString::from_smol(value.to_smolstr())),
            FieldValue::FirewallEvent(value) => {
                Value::String(ZString::from_smol(value.to_smolstr()))
            }
            FieldValue::MplsTopLabelType(value) => {
                Value::String(ZString::from_smol(value.to_smolstr()))
            }
            FieldValue::NatOriginatingAddressRealm(value) => {
                Value::String(ZString::from_smol(value.to_smolstr()))
            }
            // `FieldValue` is #[non_exhaustive]; skip variants we don't model yet.
            _ => return None,
        })
    }

    /// A v9 options scope value: an integer when it fits one (ifIndex, line
    /// card, system address), the raw bytes otherwise.
    fn scope_value(bytes: &[u8]) -> Value<'_> {
        match be_uint(bytes) {
            Some(n) => Self::encode_data_number(&DataNumber::U64(n)),
            None => Value::Bytes(Cow::Borrowed(bytes)),
        }
    }

    fn push_scope<'a>(attrs: &mut Vec<(ZString<'a>, Value<'a>)>, scope: &'a ScopeDataField) {
        let (name, bytes) = match scope {
            ScopeDataField::System(b) => (ZString::from_static("nf.scope.system"), b),
            ScopeDataField::Interface(b) => (ZString::from_static("nf.scope.interface"), b),
            ScopeDataField::LineCard(b) => (ZString::from_static("nf.scope.line_card"), b),
            ScopeDataField::NetFlowCache(b) => (ZString::from_static("nf.scope.netflow_cache"), b),
            ScopeDataField::Template(b) => (ZString::from_static("nf.scope.template"), b),
            ScopeDataField::Unknown(n, b) => (
                ZString::from_smol(format_smolstr!("nf.scope.unknown_{n}")),
                b,
            ),
        };
        attrs.push((name, Self::scope_value(bytes)));
    }

    /// Rows for the data records of a v9 flowset, from a v9 packet or a
    /// v9-style set inside IPFIX.
    fn v9_data_records(
        packet: &PacketCtx,
        flowset_id: u16,
        records: &[V9FlowRecord],
        detected: &HashMap<SamplingRateKey, SamplerRates>,
        tally: &mut Tally,
        batcher: &mut payload::LogsBuilder,
    ) {
        let flow_set_id = format_smolstr!("data:{flowset_id}");
        let mut attrs: Vec<(ZString, Value)> = Vec::new();
        for fields in records {
            tally.records += 1;
            attrs.clear();
            let mut sampling = SamplingFields::default();
            for (field, value) in fields {
                sampling.observe_v9(*field, value);
                if let Some(v) = Self::field_value_to_value(value) {
                    attrs.push((Self::extract_v9_field_str(*field), v));
                }
            }
            if attrs.is_empty() {
                tally.records_dropped += 1;
                continue;
            }
            let rate = packet.data_rate(detected, &sampling);
            packet.emit(&mut attrs, &flow_set_id, rate, batcher);
        }
    }

    /// Rows for the options records of a v9 flowset: what the exporter says
    /// about its samplers, interfaces, applications... Each one is a row of
    /// its own, with no sampling rate since it is not sampled traffic.
    fn v9_options_records(
        packet: &PacketCtx,
        flowset_id: u16,
        records: &[OptionsDataFields],
        detected: &mut HashMap<SamplingRateKey, SamplerRates>,
        tally: &mut Tally,
        batcher: &mut payload::LogsBuilder,
    ) {
        let flow_set_id = format_smolstr!("options:{flowset_id}");
        let mut attrs: Vec<(ZString, Value)> = Vec::new();
        for record in records {
            tally.records += 1;
            attrs.clear();
            let mut sampling = SamplingFields::default();
            for scope in &record.scope_fields {
                Self::push_scope(&mut attrs, scope);
            }
            for (field, value) in &record.options_fields {
                sampling.observe_v9(*field, value);
                if let Some(v) = Self::field_value_to_value(value) {
                    attrs.push((Self::extract_v9_field_str(*field), v));
                }
            }
            if attrs.is_empty() {
                tally.records_dropped += 1;
                continue;
            }
            packet.announce(detected, &sampling);
            packet.emit(&mut attrs, &flow_set_id, None, batcher);
        }
    }

    /// Rows for the data (`options: false`) or options records of an IPFIX
    /// set. IPFIX options records carry their scope fields inline, so both
    /// read the same way; options rows carry no sampling rate.
    fn ipfix_records(
        packet: &PacketCtx,
        flowset_id: u16,
        records: &[IPFixFlowRecord],
        options: bool,
        detected: &mut HashMap<SamplingRateKey, SamplerRates>,
        tally: &mut Tally,
        batcher: &mut payload::LogsBuilder,
    ) {
        let flow_set_id = if options {
            format_smolstr!("options:{flowset_id}")
        } else {
            format_smolstr!("data:{flowset_id}")
        };
        let mut attrs: Vec<(ZString, Value)> = Vec::new();
        for fields in records {
            tally.records += 1;
            attrs.clear();
            let mut sampling = SamplingFields::default();
            for (field, value) in fields {
                sampling.observe_ipfix(*field, value);
                if let Some(v) = Self::field_value_to_value(value) {
                    attrs.push((Self::extract_ipfix_field_str(*field), v));
                }
            }
            if attrs.is_empty() {
                tally.records_dropped += 1;
                continue;
            }
            let rate = if options {
                packet.announce(detected, &sampling);
                None
            } else {
                packet.data_rate(detected, &sampling)
            };
            packet.emit(&mut attrs, &flow_set_id, rate, batcher);
        }
    }

    /// `cached` holds the rates the source's shard had for a templated packet;
    /// rates its options records announce go to `sampling_detected` for
    /// [`Self::handle_packets`] to write back.
    fn parse_netflow_packet(
        &self,
        addr: SocketAddr,
        result: NetflowPacket,
        cached: Option<Arc<SamplerRates>>,
        sampling_detected: &mut HashMap<SamplingRateKey, SamplerRates>,
        tally: &mut Tally,
        batcher: &mut payload::LogsBuilder,
    ) {
        match result {
            NetflowPacket::V5(data) => {
                let capture_time =
                    DateTime::from_timestamp(data.header.unix_secs as i64, data.header.unix_nsecs);
                let sequence_number = Some(data.header.flow_sequence);
                let engine_id = Some(data.header.engine_id as u32);
                // Top 2 bits are the sampling mode; the low 14 bits are the
                // "1 in N" sampling interval.
                let sampling_rate = match data.header.sampling_interval & 0x3fff {
                    0 => None,
                    rate => Some(rate as u32),
                };
                let uptime = Some(data.header.sys_up_time as f64 / 1000.0);
                for flow in data.flowsets {
                    tally.records += 1;
                    let mut attrs: Vec<(ZString, Value)> = Vec::with_capacity(24);
                    attrs.push((
                        ZString::from_static("nf.data.srcaddr"),
                        Value::String(ZString::from_smol(flow.src_addr.to_smolstr())),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dstaddr"),
                        Value::String(ZString::from_smol(flow.dst_addr.to_smolstr())),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.nexthop"),
                        Value::String(ZString::from_smol(flow.next_hop.to_smolstr())),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.input"),
                        Value::Int64(flow.input as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.output"),
                        Value::Int64(flow.output as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dPkts"),
                        Value::Int64(flow.d_pkts as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dOctets"),
                        Value::Int64(flow.d_octets as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.First"),
                        Value::Int64(flow.first as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.Last"),
                        Value::Int64(flow.last as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.srcport"),
                        Value::Int64(flow.src_port as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dstport"),
                        Value::Int64(flow.dst_port as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.tcp_flags"),
                        Value::Int64(flow.tcp_flags as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.prot"),
                        Value::Int64(flow.protocol_number as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.tos"),
                        Value::Int64(flow.tos as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.src_as"),
                        Value::Int64(flow.src_as as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dst_as"),
                        Value::Int64(flow.dst_as as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.src_mask"),
                        Value::Int64(flow.src_mask as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dst_mask"),
                        Value::Int64(flow.dst_mask as i64),
                    ));
                    push_header(
                        &mut attrs,
                        "v5",
                        sequence_number,
                        sampling_rate,
                        engine_id,
                        None,
                        uptime,
                    );
                    batcher.push_row(Self::flow_row(addr, capture_time, &attrs));
                }
                tally.items_succeeded += 1;
            }
            NetflowPacket::V7(data) => {
                let capture_time =
                    DateTime::from_timestamp(data.header.unix_secs as i64, data.header.unix_nsecs);
                let sequence_number = Some(data.header.flow_sequence);
                let uptime = Some(data.header.sys_up_time as f64);
                for flow in data.flowsets {
                    tally.records += 1;
                    let mut attrs: Vec<(ZString, Value)> = Vec::with_capacity(24);
                    attrs.push((
                        ZString::from_static("nf.data.srcaddr"),
                        Value::String(ZString::from_smol(flow.src_addr.to_smolstr())),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dstaddr"),
                        Value::String(ZString::from_smol(flow.dst_addr.to_smolstr())),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.nexthop"),
                        Value::String(ZString::from_smol(flow.next_hop.to_smolstr())),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.input"),
                        Value::Int64(flow.input as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.output"),
                        Value::Int64(flow.output as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dPkts"),
                        Value::Int64(flow.d_pkts as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dOctets"),
                        Value::Int64(flow.d_octets as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.First"),
                        Value::Int64(flow.first as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.Last"),
                        Value::Int64(flow.last as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.srcport"),
                        Value::Int64(flow.src_port as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dstport"),
                        Value::Int64(flow.dst_port as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.flags1"),
                        Value::Int64(flow.flags_fields_valid as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.tcp_flags"),
                        Value::Int64(flow.tcp_flags as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.prot"),
                        Value::Int64(flow.protocol_number as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.tos"),
                        Value::Int64(flow.tos as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.src_as"),
                        Value::Int64(flow.src_as as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dst_as"),
                        Value::Int64(flow.dst_as as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.src_mask"),
                        Value::Int64(flow.src_mask as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.dst_mask"),
                        Value::Int64(flow.dst_mask as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.flags2"),
                        Value::Int64(flow.flags_fields_invalid as i64),
                    ));
                    attrs.push((
                        ZString::from_static("nf.data.router_sc"),
                        Value::String(ZString::from_smol(flow.router_src.to_smolstr())),
                    ));
                    push_header(&mut attrs, "v7", sequence_number, None, None, None, uptime);
                    batcher.push_row(Self::flow_row(addr, capture_time, &attrs));
                }
                tally.items_succeeded += 1;
            }
            NetflowPacket::V9(data) => {
                let packet = PacketCtx {
                    addr,
                    capture_time: DateTime::from_timestamp(data.header.unix_secs as i64, 0),
                    ty: "v9",
                    sequence_number: Some(data.header.sequence_number),
                    engine_id: Some(data.header.source_id),
                    uptime: Some(data.header.sys_up_time as f64 / 1000.0),
                    key: SamplingRateKey::new(TemplatedVersion::V9, addr, data.header.source_id),
                    cached: cached.as_deref(),
                };
                for flowset in &data.flowsets {
                    let id = flowset.header.flowset_id;
                    match &flowset.body {
                        V9FlowSetBody::Data(d) => Self::v9_data_records(
                            &packet,
                            id,
                            &d.fields,
                            sampling_detected,
                            tally,
                            batcher,
                        ),
                        V9FlowSetBody::OptionsData(d) => Self::v9_options_records(
                            &packet,
                            id,
                            &d.fields,
                            sampling_detected,
                            tally,
                            batcher,
                        ),
                        // template is consumed, don't consider this dropped
                        V9FlowSetBody::Template(_) | V9FlowSetBody::OptionsTemplate(_) => {}
                        _ => tally.records_dropped += 1,
                    }
                }
                tally.items_succeeded += 1;
            }
            NetflowPacket::IPFix(data) => {
                let packet = PacketCtx {
                    addr,
                    capture_time: DateTime::from_timestamp(data.header.export_time as i64, 0),
                    ty: "ipfix",
                    sequence_number: Some(data.header.sequence_number),
                    engine_id: Some(data.header.observation_domain_id),
                    uptime: None,
                    key: SamplingRateKey::new(
                        TemplatedVersion::IPFix,
                        addr,
                        data.header.observation_domain_id,
                    ),
                    cached: cached.as_deref(),
                };
                for flowset in &data.flowsets {
                    let id = flowset.header.header_id;
                    match &flowset.body {
                        IpfixFlowSetBody::Data(d) => Self::ipfix_records(
                            &packet,
                            id,
                            &d.fields,
                            false,
                            sampling_detected,
                            tally,
                            batcher,
                        ),
                        IpfixFlowSetBody::OptionsData(d) => Self::ipfix_records(
                            &packet,
                            id,
                            &d.fields,
                            true,
                            sampling_detected,
                            tally,
                            batcher,
                        ),
                        // v9-style sets (IDs 0/1) inside an IPFIX message
                        IpfixFlowSetBody::V9Data(d) => Self::v9_data_records(
                            &packet,
                            id,
                            &d.fields,
                            sampling_detected,
                            tally,
                            batcher,
                        ),
                        IpfixFlowSetBody::V9OptionsData(d) => Self::v9_options_records(
                            &packet,
                            id,
                            &d.fields,
                            sampling_detected,
                            tally,
                            batcher,
                        ),
                        // template is consumed, don't consider this dropped
                        IpfixFlowSetBody::Template(_)
                        | IpfixFlowSetBody::Templates(_)
                        | IpfixFlowSetBody::V9Template(_)
                        | IpfixFlowSetBody::V9Templates(_)
                        | IpfixFlowSetBody::OptionsTemplate(_)
                        | IpfixFlowSetBody::OptionsTemplates(_)
                        | IpfixFlowSetBody::V9OptionsTemplate(_)
                        | IpfixFlowSetBody::V9OptionsTemplates(_) => {}
                        _ => tally.records_dropped += 1,
                    }
                }
                tally.items_succeeded += 1;
            }
            _ => {
                tally.items_dropped += 1;
            }
        }
    }

    /// Build the per-row log fields shared across every NetFlow version: the
    /// exporter's source endpoint and the flow's capture time (as observed time).
    fn flow_row<'a, 'b: 'a>(
        addr: SocketAddr,
        capture_time: Option<DateTime<Utc>>,
        attrs: &'b Vec<(ZString<'a>, Value<'a>)>,
    ) -> LogRow<'a> {
        LogRow {
            observed_time: capture_time,
            severity: None,
            body: None,
            source_ip: Some(addr.ip()),
            source_port: Some(addr.port()),
            attrs,
        }
    }

    /// Returns the channel the workers read, starting the readers unless
    /// they run. Every worker calls this, so a worker restarted after a failed
    /// start retries it, and one restarted after a reader died (which cancels
    /// the intake) starts them afresh. `None` once the listener is closed.
    async fn ensure_started(&self) -> Result<Option<UdpReceiver>> {
        let mut state = self.state.lock().await;
        match &*state {
            State::Running(running)
                if !running.intake.is_cancelled() && !running.rx.is_disconnected() =>
            {
                return Ok(Some(running.rx.clone()));
            }
            State::Closed => return Ok(None),
            State::Running(_) | State::Idle => {}
        }
        if let State::Running(dead) = std::mem::replace(&mut *state, State::Idle) {
            warn!("udp readers stopped unexpectedly, restarting");
            dead.shutdown().await;
        }
        let running = self.start()?;
        let rx = running.rx.clone();
        *state = State::Running(running);
        Ok(Some(rx))
    }

    fn start(&self) -> Result<Running> {
        let conf = &self.conf().listener;
        let (sender, rx) = flume::bounded(conf.buffer_cap);
        let intake = CancellationToken::new();
        let udp = UdpListenerWorker::new(
            (self.conf.id, self.conf.name.to_string()),
            conf.address,
            sender,
            &intake,
            self.udp_metrics.clone(),
            conf.worker_count,
            conf.max_packet_size,
            conf.recv_buffer_size,
            Duration::from_micros(conf.spin_before_block_us),
            100,
            Duration::from_millis(100),
            conf.ip_whitelist.clone(),
            conf.ip_blacklist.clone(),
        )?;
        let mut pruner = JoinSet::new();
        pruner.spawn(
            prune_loop(
                Arc::clone(&self.shards),
                self.conf().prune_interval,
                self.conf().prune_idle_after,
                intake.clone(),
            )
            .instrument(Span::current()),
        );
        Ok(Running {
            rx,
            udp,
            intake,
            pruner,
        })
    }

    /// Parses one batch of datagrams from the readers into `batch`.
    fn handle_packets(
        &self,
        single_batch: Vec<(bytes::Bytes, SocketAddr)>,
        batch: &mut payload::LogsBuilder,
        netflow_buckets: &mut [Vec<(bytes::Bytes, SocketAddr)>],
        sflow_raw: &mut Vec<(bytes::Bytes, SocketAddr)>,
        sampling_detected: &mut HashMap<SamplingRateKey, SamplerRates>,
    ) {
        let n = Instant::now();
        sflow_raw.clear();
        for bucket in netflow_buckets.iter_mut() {
            bucket.clear();
        }
        let mut netflow_count = 0usize;
        for (packet, addr) in single_batch {
            if sflow::is_sflow(&packet) {
                sflow_raw.push((packet, addr));
            } else {
                let mut addr_wo_port = addr;
                addr_wo_port.set_port(0);
                let idx = shard_index(addr_wo_port, self.shards.len());
                netflow_buckets[idx].push((packet, addr));
                netflow_count += 1;
            }
        }

        let mut netflow_packets = Vec::with_capacity(netflow_count);
        let mut bytes_errored = 0u64;
        let mut items_errored = 0u64;
        // One lock per shard that actually saw traffic, so
        // workers on different exporters stop serializing.
        for (idx, bucket) in netflow_buckets.iter_mut().enumerate() {
            if bucket.is_empty() {
                continue;
            }
            let mut shard = self.shards[idx].lock();
            for (packet, addr) in bucket.drain(..) {
                let mut addr_wo_port = addr;
                addr_wo_port.set_port(0);
                let result = shard.parser.parse_from_source(addr_wo_port, &packet);
                if let Some(e) = &result.error {
                    trace!(peer = %addr, error = %e, "netflow parse failed");
                }
                if result.error.is_some() && result.packets.is_empty() {
                    bytes_errored += packet.len() as u64;
                    items_errored += 1;
                } else {
                    netflow_packets.reserve(result.packets.len());
                    for packet in result.packets {
                        let cached = SamplingRateKey::of(addr, &packet)
                            .and_then(|key| shard.sampling.get(&key).cloned());
                        netflow_packets.push((packet, addr, cached));
                    }
                }
            }
        }
        let mut sflow_datagrams = Vec::with_capacity(sflow_raw.len());
        for (packet, addr) in sflow_raw.drain(..) {
            match sflow_parser::parse_datagram(&packet) {
                Ok(dg) => sflow_datagrams.push((dg, addr)),
                Err(e) => {
                    trace!(peer = %addr, error = ?e, "sflow parse failed");
                    bytes_errored += packet.len() as u64;
                    items_errored += 1;
                }
            }
        }
        // Every datagram that reached us is an item; the ones
        // that failed to parse are its errored share.
        self.internal_metrics.total_items.fetch_add(
            (netflow_packets.len() + sflow_datagrams.len()) as u64 + items_errored,
            Ordering::Relaxed,
        );
        self.internal_metrics
            .total_items_errored
            .fetch_add(items_errored, Ordering::Relaxed);
        self.internal_metrics
            .total_bytes_errored
            .fetch_add(bytes_errored, Ordering::Relaxed);
        let mut tally = Tally::default();
        for (packet, addr, cached) in netflow_packets {
            self.parse_netflow_packet(addr, packet, cached, sampling_detected, &mut tally, batch);
        }
        for (dg, addr) in sflow_datagrams {
            sflow::datagram_to_logs(dg, addr, batch, &mut tally);
        }
        tally.flush(&self.internal_metrics);
        // Rates only change when an exporter is reconfigured, so this is
        // empty in steady state.
        for (key, rates) in sampling_detected.drain() {
            let idx = shard_index(SocketAddr::new(key.addr, 0), self.shards.len());
            let mut shard = self.shards[idx].lock();
            // This worker's snapshots are dropped by now, but another worker
            // may still hold one of this exporter's: then `make_mut` copies
            // the map, and that worker finishes on the old rates.
            Arc::make_mut(shard.sampling.entry(key).or_default()).extend(rates);
        }
        self.internal_metrics
            .engine_time_spent
            .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
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

/// Sweep idle sources out of every shard until cancelled.
///
/// Runs once per listener rather than once per worker: the sweep walks every
/// entry of every shard, so doing it per worker would just multiply the work.
async fn prune_loop(
    shards: Arc<[ParkingMutex<Shard>]>,
    prune_interval: Duration,
    idle_after: Duration,
    token: CancellationToken,
) {
    let mut tick = interval(prune_interval);
    // `interval` yields immediately; skip that so a restart doesn't sweep at once.
    tick.tick().await;
    loop {
        tokio::select! {
            biased;

            _ = token.cancelled() => break,
            _ = tick.tick() => {
                let (mut pruned, mut sampling_pruned) = (0usize, 0usize);
                for shard in shards.iter() {
                    // One shard at a time: the guard drops at the end of the
                    // statement, so traffic to other shards never blocks.
                    let (sources, rates) = shard.lock().prune_idle(idle_after);
                    pruned += sources;
                    sampling_pruned += rates;
                }
                if pruned > 0 || sampling_pruned > 0 {
                    debug!(pruned, sampling_pruned, "pruned idle sources");
                }
            }
        }
    }
}

#[async_trait]
impl super::Listener for Netflow {
    #[tracing::instrument(skip_all, fields(listener_id = %self.conf.id, listener = %self.conf.name))]
    async fn listen(&self, token: CancellationToken) {
        let recv = match self.ensure_started().await {
            Ok(Some(recv)) => recv,
            // Closed meanwhile.
            Ok(None) => return,
            Err(e) => {
                error!(error = %format!("{e:#}"), "failed to start udp listener");
                return;
            }
        };
        self.internal_metrics
            .total_workers
            .fetch_add(1, Ordering::Relaxed);
        info!("starting listener");
        let mut batch = payload::LogsBuilder::new();
        self.set_scope(&mut batch);
        let mut batch_tick = interval(self.conf.batch.batch_timeout);
        // Reused across iterations so bucketing costs no allocation per batch.
        let mut netflow_buckets: Vec<Vec<(bytes::Bytes, SocketAddr)>> =
            (0..self.shards.len()).map(|_| Vec::new()).collect();
        let mut sflow_raw = Vec::new();
        let mut sampling_detected = HashMap::default();
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
                res = recv.recv_async() => {
                    self.internal_metrics
                        .receive_wait_time
                        .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
                    match res {
                        Ok(single_batch) => {
                            self.handle_packets(
                                single_batch,
                                &mut batch,
                                &mut netflow_buckets,
                                &mut sflow_raw,
                                &mut sampling_detected,
                            );
                            if batch.num_items() >= self.conf.batch.batch_size {
                                self.send_batch(&mut batch).await;
                                batch_tick.reset();
                            }
                        }
                        // Disconnected: the readers stopped, or died and the
                        // supervisor restarts this worker.
                        Err(_) => {
                            debug!("transport closed");
                            break;
                        }
                    }
                }
            }
            self.internal_metrics
                .total_time_spent
                .fetch_add(n.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
        // Once the readers have stopped, what is queued is all that is left,
        // and no other worker may be around to take it.
        if recv.is_disconnected() {
            for single_batch in recv.drain() {
                self.handle_packets(
                    single_batch,
                    &mut batch,
                    &mut netflow_buckets,
                    &mut sflow_raw,
                    &mut sampling_detected,
                );
                if batch.num_items() >= self.conf.batch.batch_size {
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
        let state = std::mem::replace(&mut *self.state.lock().await, State::Closed);
        if let State::Running(running) = state {
            running.shutdown().await;
            debug!("udp readers stopped");
        }
    }

    async fn metrics(&self) -> crate::common::internal_metrics::Listener {
        let mut metrics = self
            .internal_metrics
            .get(self.conf.id, self.conf.name.to_string());
        let udp_metrics = self.udp_metrics.get();
        metrics.total_bytes = udp_metrics.total_bytes;
        metrics.total_bytes_succeeded = udp_metrics.total_bytes_succeeded;
        metrics.total_bytes_dropped = udp_metrics.total_bytes_dropped;
        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARDS: usize = 8;

    fn addr(s: &str) -> SocketAddr {
        s.parse().expect("valid socket addr")
    }

    /// A v9 header carrying `source_id`, enough for the parser to scope it.
    fn v9_header(source_id: u8) -> Vec<u8> {
        let mut data = vec![0u8; 20];
        data[1] = 0x09;
        data[19] = source_id;
        data
    }

    #[test]
    fn shard_index_is_stable_for_one_ip() {
        let a = addr("192.168.1.1:2055");
        let first = shard_index(a, SHARDS);
        for _ in 0..16 {
            assert_eq!(shard_index(a, SHARDS), first);
        }
    }

    #[test]
    fn shard_index_ignores_the_port() {
        // the hot path zeroes the port, but the key is the IP either way
        let base = shard_index(addr("10.0.0.7:2055"), SHARDS);
        assert_eq!(shard_index(addr("10.0.0.7:0"), SHARDS), base);
        assert_eq!(shard_index(addr("10.0.0.7:9995"), SHARDS), base);
    }

    #[test]
    fn shard_index_spreads_across_shards() {
        let used: std::collections::HashSet<usize> = (1..=64)
            .map(|i| shard_index(addr(&format!("10.0.0.{i}:2055")), SHARDS))
            .collect();
        assert!(used.len() > 1, "all 64 IPs landed on one shard: {used:?}");
        assert!(used.iter().all(|&i| i < SHARDS));
    }

    #[test]
    fn shard_index_is_in_range_for_one_shard() {
        assert_eq!(shard_index(addr("172.16.0.1:2055"), 1), 0);
    }

    #[test]
    fn prune_drops_only_idle_sources() {
        let shards: Vec<ParkingMutex<AutoScopedParser>> = (0..SHARDS)
            .map(|_| ParkingMutex::new(AutoScopedParser::new()))
            .collect();

        for i in 1..=4u8 {
            let a = addr(&format!("10.0.0.{i}:0"));
            let mut parser = shards[shard_index(a, SHARDS)].lock();
            let _ = parser.parse_from_source(a, &v9_header(i));
        }
        let total: usize = shards.iter().map(|s| s.lock().source_count()).sum();
        assert_eq!(total, 4);

        // freshly seen, so an hour-long idle window prunes nothing
        let pruned: usize = shards
            .iter()
            .map(|s| s.lock().prune_idle_sources(Duration::from_secs(3600)))
            .sum();
        assert_eq!(pruned, 0);
        let total: usize = shards.iter().map(|s| s.lock().source_count()).sum();
        assert_eq!(total, 4);

        // a zero idle window makes every source eligible
        let pruned: usize = shards
            .iter()
            .map(|s| s.lock().prune_idle_sources(Duration::ZERO))
            .sum();
        assert_eq!(pruned, 4);
        let total: usize = shards.iter().map(|s| s.lock().source_count()).sum();
        assert_eq!(total, 0);
    }

    fn generic_rate(rate: u32) -> Arc<SamplerRates> {
        Arc::new([(GENERIC_SAMPLER, rate)].into_iter().collect())
    }

    #[test]
    fn prune_drops_sampling_with_its_source() {
        let a = addr("10.0.0.1:0");
        let mut shard = Shard::new(AutoScopedParser::new());
        let _ = shard.parser.parse_from_source(a, &v9_header(1));
        let live = SamplingRateKey::new(TemplatedVersion::V9, a, 1);
        let orphan = SamplingRateKey::new(TemplatedVersion::V9, addr("10.0.0.2:0"), 2);
        shard.sampling.insert(live, generic_rate(100));
        shard.sampling.insert(orphan, generic_rate(200));

        // the source is fresh, so only the rate nothing holds templates for goes
        assert_eq!(shard.prune_idle(Duration::from_secs(3600)), (0, 1));
        assert_eq!(
            shard
                .sampling
                .get(&live)
                .and_then(|r| r.get(&GENERIC_SAMPLER).copied()),
            Some(100)
        );
        assert!(!shard.sampling.contains_key(&orphan));

        assert_eq!(shard.prune_idle(Duration::ZERO), (1, 1));
        assert!(shard.sampling.is_empty());
    }

    /// A v9 packet from `source_id` 7: optionally a template (256: IPv4 source
    /// address, sampling interval), then a data flowset of `records` flows
    /// each reporting `interval`.
    fn v9_sampled(with_template: bool, records: u8, interval: u32) -> Vec<u8> {
        let mut data = Vec::new();
        let count = records as u16 + with_template as u16;
        data.extend_from_slice(&9u16.to_be_bytes());
        data.extend_from_slice(&count.to_be_bytes());
        data.extend_from_slice(&[0; 12]); // uptime, unix_secs, sequence
        data.extend_from_slice(&7u32.to_be_bytes());
        if with_template {
            for v in [0u16, 16, 256, 2, 8, 4, 34, 4] {
                data.extend_from_slice(&v.to_be_bytes());
            }
        }
        data.extend_from_slice(&256u16.to_be_bytes());
        data.extend_from_slice(&(4 + 8 * records as u16).to_be_bytes());
        for i in 0..records {
            data.extend_from_slice(&[192, 168, 0, i]);
            data.extend_from_slice(&interval.to_be_bytes());
        }
        data
    }

    /// One emitted row's attributes, by key, with the OTLP JSON value wrapper
    /// (`{"intValue": ..}`, `{"stringValue": ..}`) left on.
    type Row = std::collections::HashMap<String, serde_json::Value>;

    fn int(value: &serde_json::Value) -> Option<i64> {
        match &value["intValue"] {
            serde_json::Value::String(s) => s.parse().ok(),
            other => other.as_i64(),
        }
    }

    fn string(value: &serde_json::Value) -> Option<&str> {
        value["stringValue"].as_str()
    }

    /// Every log record in `value`, depth first.
    fn rows(value: &serde_json::Value, out: &mut Vec<Row>) {
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
                map.values().for_each(|v| rows(v, out));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| rows(v, out)),
            _ => {}
        }
    }

    /// Every row's `nf.sampling_rate`, skipping the rows without one.
    fn sampling_rates(rows: &[Row]) -> Vec<i64> {
        rows.iter()
            .filter_map(|r| r.get("nf.sampling_rate").and_then(int))
            .collect()
    }

    /// A listener plus the per-worker scratch [`Netflow::handle_packets`] takes.
    struct Harness {
        nf: Netflow,
        buckets: Vec<Vec<(bytes::Bytes, SocketAddr)>>,
        sflow_raw: Vec<(bytes::Bytes, SocketAddr)>,
        detected: HashMap<SamplingRateKey, SamplerRates>,
    }

    impl Harness {
        fn new() -> Self {
            use crate::common::internal_metrics;
            use arc_swap::ArcSwap;

            let mut conf: config::listener::Listener = serde_yaml_ng::from_str(
                "id: 019b77b3-df79-70a2-ab01-f43b2cf723a4\nname: nf\ntype: netflow\nconfig: {}\n",
            )
            .unwrap();
            conf.validate().unwrap();
            let (_admin, sender) = crate::chan::new(
                Duration::from_secs(3600),
                Arc::new(ArcSwap::from_pointee((
                    std::time::Instant::now(),
                    internal_metrics::Channel::default(),
                ))),
            );
            let nf = Netflow::new(conf, sender, None, cache::Cache::new(), None);
            Self {
                buckets: vec![Vec::new(); nf.shards.len()],
                sflow_raw: Vec::new(),
                detected: HashMap::default(),
                nf,
            }
        }

        /// Handles `packets` from `exporter` as one batch and returns its rows.
        fn run(&mut self, exporter: SocketAddr, packets: Vec<Vec<u8>>) -> Vec<Row> {
            let mut batch = payload::LogsBuilder::new();
            self.nf.set_scope(&mut batch);
            let packets = packets
                .into_iter()
                .map(|p| (bytes::Bytes::from(p), exporter))
                .collect();
            self.nf.handle_packets(
                packets,
                &mut batch,
                &mut self.buckets,
                &mut self.sflow_raw,
                &mut self.detected,
            );
            let finished = batch.finish();
            let json: serde_json::Value = serde_json::from_slice(
                finished
                    .serialize_as(config::Format::JsonOtlpBatch)
                    .unwrap()[0]
                    .as_ref(),
            )
            .unwrap();
            let mut out = Vec::new();
            rows(&json, &mut out);
            assert_eq!(out.len(), finished.num_items());
            out
        }

        /// The rates the shard holds for `key`.
        fn shard_rates(&self, key: &SamplingRateKey) -> Option<SamplerRates> {
            let exporter = SocketAddr::new(key.addr, 0);
            self.nf.shards[shard_index(exporter, self.nf.shards.len())]
                .lock()
                .sampling
                .get(key)
                .map(|r| (**r).clone())
        }
    }

    #[tokio::test]
    async fn inline_rate_applies_to_its_record() {
        let mut h = Harness::new();
        let exporter = addr("10.0.0.9:2055");
        let key = SamplingRateKey::new(TemplatedVersion::V9, exporter, 7);
        let run = |h: &mut Harness, packets: Vec<Vec<u8>>| {
            let rows = h.run(exporter, packets);
            (rows.len(), sampling_rates(&rows))
        };

        let (rows, rates) = run(
            &mut h,
            vec![v9_sampled(true, 2, 100), v9_sampled(false, 2, 100)],
        );
        assert_eq!((rows, rates), (4, vec![100, 100, 100, 100]));
        assert_eq!(
            h.nf.internal_metrics.total_records.load(Ordering::Relaxed),
            4
        );
        // the template flowset is consumed by the parser, not dropped
        assert_eq!(
            h.nf.internal_metrics
                .total_records_dropped
                .load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            h.nf.internal_metrics
                .total_items_succeeded
                .load(Ordering::Relaxed),
            2
        );

        // each record carries its own rate, so a change applies at once
        let (rows, rates) = run(&mut h, vec![v9_sampled(false, 1, 200)]);
        assert_eq!((rows, rates), (1, vec![200]));
        // and nothing is remembered: only options records announce rates
        assert_eq!(h.shard_rates(&key), None);
    }

    fn be16(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    /// Appends a v9 flowset / IPFIX set `id` holding `body`, padded to 4 bytes.
    fn set(out: &mut Vec<u8>, id: u16, body: &[u8]) {
        let pad = (4 - body.len() % 4) % 4;
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&((4 + body.len() + pad) as u16).to_be_bytes());
        out.extend_from_slice(body);
        out.extend(std::iter::repeat_n(0, pad));
    }

    /// A v9 packet from `source_id` 7 around `flowsets`.
    fn v9_packet(flowsets: &[u8]) -> Vec<u8> {
        let mut data = be16(&[9, 100]); // version, count (an upper bound is enough)
        data.extend_from_slice(&[0; 12]); // uptime, unix_secs, sequence
        data.extend_from_slice(&7u32.to_be_bytes());
        data.extend_from_slice(flowsets);
        data
    }

    /// An IPFIX message from observation domain 7 around `sets`.
    fn ipfix_packet(sets: &[u8]) -> Vec<u8> {
        let mut data = be16(&[10, (16 + sets.len()) as u16]);
        data.extend_from_slice(&[0; 8]); // export time, sequence
        data.extend_from_slice(&7u32.to_be_bytes());
        data.extend_from_slice(sets);
        data
    }

    /// v9 data template 256: IPv4 source address, `FLOW_SAMPLER_ID`/4.
    fn v9_sampler_template(out: &mut Vec<u8>) {
        set(out, 0, &be16(&[256, 2, 8, 4, 48, 4]));
    }

    /// v9 data flowset 256: one record per sampler ID.
    fn v9_sampler_data(out: &mut Vec<u8>, samplers: &[u32]) {
        let body: Vec<u8> = samplers
            .iter()
            .enumerate()
            .flat_map(|(i, id)| [[192, 168, 0, i as u8], id.to_be_bytes()].concat())
            .collect();
        set(out, 256, &body);
    }

    #[tokio::test]
    async fn v9_options_set_rates_per_sampler() {
        let mut h = Harness::new();
        let exporter = addr("10.0.0.9:2055");
        let key = SamplingRateKey::new(TemplatedVersion::V9, exporter, 7);

        // FNF-style sampler table: options template 257, scope System/4,
        // FLOW_SAMPLER_ID/4, FLOW_SAMPLER_RANDOM_INTERVAL/4
        let mut first = Vec::new();
        set(&mut first, 1, &be16(&[257, 4, 8, 1, 4, 48, 4, 50, 4]));
        let mut options = Vec::new();
        for (id, interval) in [(1u32, 100u32), (2, 500)] {
            options.extend_from_slice(&[10, 0, 0, 9]);
            options.extend_from_slice(&id.to_be_bytes());
            options.extend_from_slice(&interval.to_be_bytes());
        }
        set(&mut first, 257, &options);
        v9_sampler_template(&mut first);
        v9_sampler_data(&mut first, &[1, 2, 9, 1]);

        let rows = h.run(exporter, vec![v9_packet(&first)]);
        assert_eq!(rows.len(), 6);
        let (opts, data) = rows.split_at(2);
        for row in opts {
            assert_eq!(
                row.get("nf.flow_set_id").and_then(string),
                Some("options:257")
            );
            assert_eq!(row.get("nf.scope.system").and_then(int), Some(0x0a000009));
            assert!(!row.contains_key("nf.sampling_rate"));
        }
        assert!(
            data.iter()
                .all(|r| r.get("nf.flow_set_id").and_then(string) == Some("data:256"))
        );
        // the options flowset applies to the data after it; sampler 9 is
        // unknown and there is no generic rate yet
        assert_eq!(sampling_rates(data), vec![100, 500, 100]);
        assert!(!data[2].contains_key("nf.sampling_rate"));

        // traditional sampling: SAMPLING_INTERVAL with no sampler ID is the
        // generic rate, the fallback for samplers nothing announced
        let mut second = Vec::new();
        set(&mut second, 1, &be16(&[258, 4, 4, 1, 4, 34, 4]));
        set(
            &mut second,
            258,
            &[[10, 0, 0, 9], 64u32.to_be_bytes()].concat(),
        );
        v9_sampler_data(&mut second, &[9, 2]);
        let rows = h.run(exporter, vec![v9_packet(&second)]);
        assert_eq!(sampling_rates(&rows), vec![64, 500]);

        let want: SamplerRates = [(GENERIC_SAMPLER, 64), (1, 100), (2, 500)]
            .into_iter()
            .collect();
        assert_eq!(h.shard_rates(&key), Some(want));
        assert_eq!(
            h.nf.internal_metrics.total_records.load(Ordering::Relaxed),
            9
        );
    }

    #[tokio::test]
    async fn ipfix_options_set_rates_per_selector() {
        let mut h = Harness::new();
        let exporter = addr("10.0.0.10:4739");

        let mut sets = Vec::new();
        // RFC 5476 selector report: options template 300, scope selectorId,
        // samplingPacketInterval, samplingPacketSpace
        set(&mut sets, 3, &be16(&[300, 3, 1, 302, 4, 305, 4, 306, 4]));
        set(
            &mut sets,
            300,
            &[5u32.to_be_bytes(), 1u32.to_be_bytes(), 99u32.to_be_bytes()].concat(),
        );
        // data template 301: sourceIPv4Address, selectorId
        set(&mut sets, 2, &be16(&[301, 2, 8, 4, 302, 4]));
        set(
            &mut sets,
            301,
            &[
                [192, 168, 0, 1],
                5u32.to_be_bytes(),
                [192, 168, 0, 2],
                6u32.to_be_bytes(),
            ]
            .concat(),
        );

        let rows = h.run(exporter, vec![ipfix_packet(&sets)]);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0].get("nf.flow_set_id").and_then(string),
            Some("options:300")
        );
        assert_eq!(rows[0].get("nf.data.selectorId").and_then(int), Some(5));
        // one packet selected out of every 1 + 99
        assert_eq!(sampling_rates(&rows), vec![100]);
        assert!(!rows[2].contains_key("nf.sampling_rate"));
    }

    #[tokio::test]
    async fn ipfix_carries_v9_sets() {
        let mut h = Harness::new();
        let exporter = addr("10.0.0.11:4739");

        let mut sets = Vec::new();
        // v9-style options template (set 1) and data template (set 0)
        set(&mut sets, 1, &be16(&[257, 4, 8, 1, 4, 48, 4, 50, 4]));
        set(
            &mut sets,
            257,
            &[[10, 0, 0, 11], 1u32.to_be_bytes(), 1000u32.to_be_bytes()].concat(),
        );
        v9_sampler_template(&mut sets);
        v9_sampler_data(&mut sets, &[1]);

        let rows = h.run(exporter, vec![ipfix_packet(&sets)]);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| r.get("nf.type").and_then(string) == Some("ipfix"))
        );
        assert_eq!(
            rows[0].get("nf.flow_set_id").and_then(string),
            Some("options:257")
        );
        assert_eq!(
            rows[0].get("nf.scope.system").and_then(int),
            Some(0x0a00000b)
        );
        assert_eq!(
            rows[1].get("nf.data.FLOW_SAMPLER_ID").and_then(int),
            Some(1)
        );
        assert_eq!(sampling_rates(&rows), vec![1000]);
    }

    #[tokio::test]
    async fn generic_rate_from_options_without_id() {
        let mut h = Harness::new();
        let exporter = addr("10.0.0.14:4739");

        let mut sets = Vec::new();
        // a selector report scoped by observation domain, naming no selector:
        // one packet out of every 1 + 9
        set(&mut sets, 3, &be16(&[300, 3, 1, 149, 4, 305, 4, 306, 4]));
        set(
            &mut sets,
            300,
            &[7u32.to_be_bytes(), 1u32.to_be_bytes(), 9u32.to_be_bytes()].concat(),
        );
        // data template 301: sourceIPv4Address, no selector
        set(&mut sets, 2, &be16(&[301, 1, 8, 4]));
        set(&mut sets, 301, &[192, 168, 0, 1, 192, 168, 0, 2]);

        let rows = h.run(exporter, vec![ipfix_packet(&sets)]);
        assert_eq!(rows.len(), 3);
        assert_eq!(sampling_rates(&rows), vec![10, 10]);
    }

    #[test]
    fn sampling_fields_rate() {
        let rate = |f: SamplingFields| f.rate();
        assert_eq!(
            rate(SamplingFields {
                population: Some(1000),
                size: Some(10),
                packet_interval: Some(3),
                ..Default::default()
            }),
            Some(100)
        );
        assert_eq!(
            rate(SamplingFields {
                packet_interval: Some(2),
                packet_space: Some(98),
                ..Default::default()
            }),
            Some(50)
        );
        assert_eq!(
            rate(SamplingFields {
                probability: Some(0.01),
                ..Default::default()
            }),
            Some(100)
        );
        assert_eq!(
            rate(SamplingFields {
                probability: Some(1.5),
                ..Default::default()
            }),
            None
        );
        assert_eq!(
            rate(SamplingFields {
                random_interval: Some(256),
                interval: Some(10),
                ..Default::default()
            }),
            Some(256)
        );
        assert_eq!(
            rate(SamplingFields {
                interval: Some(0),
                ..Default::default()
            }),
            None
        );
        assert_eq!(
            rate(SamplingFields {
                interval: Some(u64::MAX),
                ..Default::default()
            }),
            None
        );
    }

    #[test]
    fn scope_value_decodes_short_integers() {
        assert!(matches!(
            Netflow::scope_value(&[0, 0, 0, 7]),
            Value::Int64(7)
        ));
        assert!(matches!(Netflow::scope_value(&[1]), Value::Int64(1)));
        // past i64::MAX, encode_data_number's string form
        assert!(matches!(Netflow::scope_value(&[0xff; 8]), Value::String(_)));
        assert!(matches!(
            Netflow::scope_value(&[1; 9]),
            Value::Bytes(b) if b.len() == 9
        ));
    }

    /// The transport's lifecycle, as the listener's `Worker` runs it: the
    /// syslog close tests' shape, over v9 datagrams that each carry their
    /// template, so any reader and worker turns one into exactly one row.
    mod lifecycle {
        use super::*;
        use crate::listener::test_util::{self, Port, item_for};
        use crate::listener::{DRAIN_TIMEOUT, Listener as _};

        const DATAGRAMS: usize = 1000;

        type Harness = test_util::Harness<Netflow>;

        impl test_util::Transport for Netflow {
            async fn running(&self) -> bool {
                matches!(
                    &*self.state.lock().await,
                    State::Running(running)
                        if !running.intake.is_cancelled() && !running.rx.is_disconnected()
                )
            }
        }

        fn datagram() -> Vec<u8> {
            v9_sampled(true, 1, 100)
        }

        async fn start(buffer_cap: usize) -> Harness {
            Harness::start(Port::Udp, DRAIN_TIMEOUT, |addr, sender| {
                let listener_id = Uuid::now_v7();
                let conf = config::listener::Listener::new(
                    listener_id,
                    "netflow-close".into(),
                    test_util::WORKERS,
                    config::Batch {
                        batch_size: 10,
                        batch_timeout: Duration::from_millis(50),
                    },
                    config::listener::Config::Netflow(config::listener::Netflow {
                        listener: config::listener::UdpListener {
                            address: addr,
                            max_packet_size: 2048,
                            buffer_cap,
                            recv_buffer_size: 1 << 20,
                            spin_before_block_us: 0,
                            worker_count: 1,
                            ip_whitelist: Vec::new(),
                            ip_blacklist: Vec::new(),
                        },
                        max_sources: 16,
                        prune_interval: Duration::from_secs(300),
                        prune_idle_after: Duration::from_secs(3600),
                    }),
                    false,
                );
                let nf = Netflow::new(conf.clone(), sender, None, item_for(listener_id), None);
                (conf, nf)
            })
            .await
        }

        fn send(h: &Harness, count: usize) {
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let datagram = datagram();
            for _ in 0..count {
                socket.send_to(&datagram, h.addr).unwrap();
            }
        }

        async fn kill_readers(h: &Harness) {
            if let State::Running(running) = &*h.listener.state.lock().await {
                running.udp.kill_readers();
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn netflow_close_delivers_everything_read() {
            // Room for every batch, so the readers take all the datagrams off
            // the socket before the close.
            let h = start(DATAGRAMS).await;
            send(&h, DATAGRAMS);
            h.wait_for_intake((DATAGRAMS * datagram().len()) as u64)
                .await;
            let (_, rows) = h.close(true).await;
            assert_eq!(rows, Some(DATAGRAMS));
        }

        /// N11: readers blocked on a full channel when the close comes hand
        /// over what they hold.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn netflow_close_with_small_buffer_cap_delivers_everything() {
            let h = start(1).await;
            send(&h, DATAGRAMS);
            h.wait_for_stalled_intake().await;
            let listener = h.listener.clone();
            let (_, rows) = h.close(true).await;
            let m = listener.metrics().await;
            assert_eq!(m.total_bytes_dropped, 0);
            assert_eq!(
                rows,
                Some(m.total_bytes_succeeded as usize / datagram().len())
            );
            assert_eq!(m.total_items_succeeded as usize, rows.unwrap());
        }

        /// Workers come and go (a crash and restart, a scale to zero) while
        /// the readers keep their sockets.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn scale_to_zero_keeps_socket() {
            let h = start(DATAGRAMS).await;
            let (rows, _consumer) = h.consume();
            send(&h, 1);
            Harness::wait_for_rows(&rows, 1).await;
            h.worker.adjust(0).await;
            assert_eq!(h.workers().await, 0);
            assert!(h.running().await);
            send(&h, 1);
            h.restart_workers().await;
            Harness::wait_for_rows(&rows, 2).await;
            h.close(false).await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn readers_dying_restart_transport() {
            let h = start(DATAGRAMS).await;
            let (rows, _consumer) = h.consume();
            kill_readers(&h).await;
            h.wait_for_no_workers().await;
            h.restart_workers().await;
            send(&h, 1);
            Harness::wait_for_rows(&rows, 1).await;
            h.close(false).await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn closed_stays_closed() {
            let h = start(DATAGRAMS).await;
            let (addr, listener) = (h.addr, h.listener.clone());
            h.close(false).await;
            // A worker spawned just before the close must not start it again.
            assert!(listener.ensure_started().await.unwrap().is_none());
            std::net::UdpSocket::bind(addr).expect("port released");
        }

        /// The readers' bytes used to be read off the running transport, so
        /// they went back to 0 on a close and started over on a restart.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn byte_metrics_survive_close() {
            let h = start(DATAGRAMS).await;
            let (rows, _consumer) = h.consume();
            let one = datagram().len() as u64;
            send(&h, 1);
            Harness::wait_for_rows(&rows, 1).await;
            kill_readers(&h).await;
            h.wait_for_no_workers().await;
            h.restart_workers().await;
            send(&h, 1);
            Harness::wait_for_rows(&rows, 2).await;
            h.wait_for_intake(2 * one).await;
            let listener = h.listener.clone();
            h.close(false).await;
            let m = listener.metrics().await;
            assert_eq!(m.total_bytes, 2 * one);
            assert_eq!(m.total_bytes_succeeded, 2 * one);
        }
    }
}
