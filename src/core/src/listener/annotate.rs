// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::payload::attr::{
    AttrLocation, StrCol, attribute_filter_mask, collect_parent_ids, get_mask_by_id, get_record,
    get_record_attr, struct_id_column,
};
use crate::{cache, config, payload};
use anyhow::Result;
use arrow::array::{
    Array, ArrayRef, AsArray, BooleanArray, RecordBatch, StringArray, StructArray, UInt16Array,
};
use arrow::compute::kernels;
use arrow::datatypes::UInt16Type;
use datafusion_expr::ColumnarValue;
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::encode::record::attributes::AttributesRecordBatchBuilder;
use otel_arrow_dfe_pdata::otap::filter::{IdBitmapPool, filter_otap_batch};
use otel_arrow_dfe_pdata::otap::transform::upsert_attributes::{
    AttributeUpsert, upsert_attributes,
};
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::consts;
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::{SmallVec, smallvec};
use smol_str::SmolStr;
use std::fmt::Write;
use std::net::IpAddr;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Default)]
struct ItemIdResourceMap {
    item_to_parent_id: FxHashMap<Uuid, u16>,
    parent_id_to_item: FxHashMap<u16, Uuid>,
    max_parent_id: u16,
}

pub struct ListenerAnnotate {
    /// Records owned by a remote node, keyed by that node id.
    pub for_forward: Vec<payload::ForwarderPayload>,
    /// Records this node processes locally (owned, unowned, or unmatched).
    /// keyed by the processor id
    pub for_process: Vec<payload::ProcessorPayload>,
}

#[derive(Clone)]
enum Annotate {
    ItemId(Uuid),
    ItemData(cache::listener::ItemData),
    Complete(Box<Complete>),
}

/// A record resolved to an item this node owns: the processors to route it to, the
/// resource attributes to stamp on it, and the item's health counters.
#[derive(Clone)]
struct Complete {
    processors: SmallVec<[Uuid; 4]>,
    resource: payload::Resource,
    /// `None` when the item isn't in the item cache — a listener item whose host
    /// has no `monitored_by` yet, which the leader's rebalance fixes on its next
    /// pass. Ingest for those rows goes unrecorded rather than to the wrong item.
    stats: Option<Arc<cache::item::Stats>>,
}

pub fn item_listener_annotate(
    pl: &payload::Payload,
    conf: Option<&config::listener::Listener>,
    conf_cache: &cache::Cache,
    node_id: Option<Uuid>,
    disable_forwarding: bool,
) -> Result<ListenerAnnotate> {
    // Row count, not item count: for metrics `num_items()` counts data points, so
    // it would mis-size every per-record vector below (and the masks they feed to
    // `filter_otap_batch`) whenever a metric row carries other than exactly one.
    if pl.num_rows() == 0 {
        return Ok(ListenerAnnotate {
            for_forward: vec![],
            for_process: vec![
                (payload::ProcessorPayload {
                    processor_id: None,
                    payload: pl.clone(),
                }),
            ],
        });
    };
    let mut item_annotates: Vec<Option<Annotate>> = vec![None; pl.num_rows()];
    // Records may already carry a resource item_id (e.g. forwarded from another
    // node). Seed item_ids from them so the matching loop below only assigns the
    // still-unmatched (None) records and we can reuse their resource ids later.
    let existing_item_resource_map = item_id_resource_build_map(pl)?;
    seed_existing_item_ids(pl, &existing_item_resource_map, &mut item_annotates)?;
    if let Some(conf) = conf
        && let lis_read = conf_cache.listener.read()
        && let Some(items) = lis_read.get_items(&conf.id)
    {
        for item in items {
            let config::item::Config::Listener(item_conf) = &item.item.config else {
                continue;
            };
            // short-circuit if all item_ids are already collected
            if item_annotates.iter().all(Option::is_some) {
                break;
            }
            let masks = match &item_conf.match_strategy {
                config::item::ListenerItemMatchStrategy::None => {
                    BooleanArray::from(vec![true; item_annotates.len()])
                }
                config::item::ListenerItemMatchStrategy::EndpointSourceIp => {
                    let Some(ep_id) = &item.item.endpoint_id else {
                        continue;
                    };
                    let Some(ip) = lis_read.get_endpoint_ip(ep_id) else {
                        continue;
                    };
                    let mut set = FxHashSet::default();
                    match_by_ip(pl, *ip, &mut set)?;
                    get_mask_by_id(pl, AttrLocation::Record, &set)?
                }
                config::item::ListenerItemMatchStrategy::SourceIps(ips) => {
                    let mut set = FxHashSet::default();
                    for ip in ips {
                        match_by_ip(pl, *ip, &mut set)?;
                    }
                    get_mask_by_id(pl, AttrLocation::Record, &set)?
                }
                config::item::ListenerItemMatchStrategy::AttributeMatch(filter) => {
                    attribute_filter_mask(pl, filter)?
                }
            };
            for (i, mask) in masks.iter().enumerate() {
                if item_annotates[i].is_none() && mask == Some(true) {
                    item_annotates[i] = Some(Annotate::ItemData(item.clone()));
                }
            }
        }
    };
    if item_annotates.iter().all(Option::is_none) {
        return Ok(ListenerAnnotate {
            for_forward: vec![],
            for_process: vec![
                (payload::ProcessorPayload {
                    processor_id: None,
                    payload: pl.clone(),
                }),
            ],
        });
    };
    // completing the annotation
    {
        let item_read = conf_cache.item.read();
        let ep_read = conf_cache.endpoint.read();
        let host_cache = conf_cache.host.read();
        for annotate in item_annotates.iter_mut() {
            let new_annotate = match annotate {
                Some(Annotate::ItemId(id)) => {
                    let Some(item) = item_read.get(id) else {
                        continue;
                    };
                    let ep = item.endpoint_id.and_then(|id| ep_read.get(&id));
                    let host = host_cache.get(&item.host_id);
                    Some(Annotate::Complete(Box::new(Complete {
                        processors: item.processors.clone(),
                        stats: item_read.stats(id),
                        resource: payload::Resource {
                            host_id: host.map(|host| host.id).unwrap_or(Uuid::nil()),
                            host_name: host.map(|host| host.name.clone()).unwrap_or_default(),
                            host_timezone: host.and_then(|host| host.timezone.clone()),
                            endpoint_id: ep.map(|ep| ep.id),
                            endpoint_name: ep.map(|ep| ep.name.clone()),
                            endpoint_type: ep.map(|ep| {
                                SmolStr::new_static(
                                    config::endpoint::EndpointKind::from(&ep.config).into(),
                                )
                            }),
                            item_id: *id,
                            item_name: item.name.clone(),
                        },
                    })))
                }
                Some(Annotate::ItemData(data)) if monitored_by_us(data.monitored_by, node_id) => {
                    let ep = data.item.endpoint_id.and_then(|id| ep_read.get(&id));
                    let host = host_cache.get(&data.item.host_id);
                    Some(Annotate::Complete(Box::new(Complete {
                        processors: data.item.processors.clone(),
                        stats: item_read.stats(&data.item.id),
                        resource: payload::Resource {
                            host_id: host.map(|host| host.id).unwrap_or(Uuid::nil()),
                            host_name: host.map(|host| host.name.clone()).unwrap_or_default(),
                            host_timezone: host.and_then(|host| host.timezone.clone()),
                            endpoint_id: ep.map(|ep| ep.id),
                            endpoint_name: ep.map(|ep| ep.name.clone()),
                            endpoint_type: ep.map(|ep| {
                                SmolStr::new_static(
                                    config::endpoint::EndpointKind::from(&ep.config).into(),
                                )
                            }),
                            item_id: data.item.id,
                            item_name: data.item.name.clone(),
                        },
                    })))
                }
                _ => continue,
            };
            *annotate = new_annotate;
        }
    };

    // ---------------------------------------------------------------------
    // Attach resource attributes (via upsert) and split by destination.
    //
    // After the completion loop each entry is one of:
    //   Complete(res) — fresh local match (new parent_id, insert all attrs) or a
    //                   seeded record this node owns (reuse parent_id, insert all
    //                   *except* patrolli.item.id which is already present);
    //   ItemData      — matched but owned by another node → forward, carrying only
    //                   patrolli.item.id (the owner fills in the rest);
    //   ItemId        — seeded record whose item is unknown here → pass through
    //                   untouched (its patrolli.item.id is already present);
    //   None          — unmatched → pass through with no resource.
    //
    // Many records can resolve to the SAME item, and therefore share one resource
    // parent_id. We collect each resource's attributes only ONCE (keyed by the
    // parent_ids we've already filled) — otherwise the output ResourceAttrs batch
    // gains one duplicate row per record (host.name, service.name, … repeated
    // thousands of times). The only key that can already exist in the inbound
    // batch is patrolli.item.id and we never re-insert it, so every upsert is a pure
    // append (existing_key_mask all false).
    let mut item_to_parent = existing_item_resource_map.item_to_parent_id.clone();
    let mut next_parent_id = if item_to_parent.is_empty() {
        0u16
    } else {
        existing_item_resource_map.max_parent_id + 1
    };
    let mut collectors = AttrCollectors::default();
    let mut row_parent_id: Vec<Option<u16>> = vec![None; pl.num_rows()];
    let mut row_dest: Vec<Destination> = vec![Destination::Process(smallvec![]); pl.num_rows()];
    // Resource parent_ids whose attributes have already been collected this batch.
    let mut filled_parent_ids: FxHashSet<u16> = FxHashSet::default();
    // Whether any record resolved to a Complete/ItemData annotation (i.e. needs a
    // resource.id rewrite and/or forwarding). ItemId/None alone are a passthrough.
    let mut any_annotated = false;
    // Rows received per item, for [`cache::item::Stats`]. Only items this node owns
    // get an entry: a row bound for another node is that node's to count, once it
    // annotates the forwarded copy. That keeps every item's counters on exactly one
    // node, so the stats API concatenates instead of merging.
    // The name rides along for the state-transition log.
    let mut ingest: FxHashMap<Uuid, (Arc<cache::item::Stats>, SmolStr, u64)> = FxHashMap::default();
    for (i, annotate) in item_annotates.into_iter().enumerate() {
        match annotate {
            Some(Annotate::Complete(res)) => {
                any_annotated = true;
                let (pid, _) = alloc_parent_id(
                    &mut item_to_parent,
                    &mut next_parent_id,
                    res.resource.item_id,
                );
                row_parent_id[i] = Some(pid);
                if let Some(stats) = &res.stats {
                    let tally = ingest
                        .entry(res.resource.item_id)
                        .or_insert_with(|| (stats.clone(), res.resource.item_name.clone(), 0));
                    tally.2 += 1;
                }
                // Fill this resource's attributes the first time we see its
                // parent_id; later records for the same item reuse the same
                // resource row. Skip patrolli.item.id when the inbound batch already
                // carries it at this resource (a seeded record).
                if filled_parent_ids.insert(pid) {
                    let item_id_present = existing_item_resource_map
                        .parent_id_to_item
                        .contains_key(&pid);
                    push_resource(&mut collectors, pid, &res.resource, !item_id_present);
                }
                row_dest[i] = Destination::Process(res.processors);
            }
            Some(Annotate::ItemData(data)) => {
                any_annotated = true;
                let (pid, _) =
                    alloc_parent_id(&mut item_to_parent, &mut next_parent_id, data.item.id);
                row_parent_id[i] = Some(pid);
                row_dest[i] = data
                    .monitored_by
                    .map(Destination::Forward)
                    .unwrap_or(Destination::Process(smallvec![]));
                // Carry only patrolli.item.id, once per resource, and only when it
                // isn't already present at this resource (the owner node fills in
                // everything else).
                if filled_parent_ids.insert(pid)
                    && !existing_item_resource_map
                        .parent_id_to_item
                        .contains_key(&pid)
                {
                    collectors.push(
                        payload::ResourceAttrKey::ItemId,
                        pid,
                        data.item.id.as_hyphenated().to_string(),
                    );
                }
            }
            // Seeded record we couldn't resolve: keep whatever resource it points at.
            Some(Annotate::ItemId(id)) => {
                row_parent_id[i] = existing_item_resource_map
                    .item_to_parent_id
                    .get(&id)
                    .copied();
            }
            None => {}
        }
    }

    // Recorded here, at the match, rather than after the split below: the rows did
    // arrive for this item, and whether they then reach a sink is a separate fact
    // that `total_dropped` answers. Same split the monitor makes between
    // `record_success` and the batcher's `add_dropped`.
    if !ingest.is_empty() {
        let now_ms = crate::common::now().timestamp_millis();
        for (item_id, (stats, name, rows)) in &ingest {
            if let Some(t) = stats.record_ingest(now_ms, *rows) {
                t.log(*item_id, name, None);
            }
        }
    }

    // Only seeded/unmatched records → pass through untouched.
    if !any_annotated {
        return Ok(ListenerAnnotate {
            for_forward: vec![],
            for_process: vec![
                (payload::ProcessorPayload {
                    processor_id: None,
                    payload: pl.clone(),
                }),
            ],
        });
    }

    // Build the new ResourceAttrs by upserting the collected attributes into the
    // existing batch (or an empty one when the inbound payload carries none).
    let empty_ra;
    let existing_ra = match pl.data.get(ArrowPayloadType::ResourceAttrs) {
        Some(ra) => ra,
        None => {
            empty_ra = AttributesRecordBatchBuilder::<u16>::new()
                .finish()
                .map_err(|e| anyhow::anyhow!("build empty resource attrs: {e}"))?;
            &empty_ra
        }
    };
    let num_existing = existing_ra.num_rows();
    let upserts = collectors
        .rows
        .iter()
        .map(|(key, rows)| AttributeUpsert {
            attrs_key: key,
            existing_key_mask: BooleanArray::from(vec![false; num_existing]),
            new_values: ColumnarValue::Array(Arc::new(StringArray::from(
                rows.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(),
            )) as ArrayRef),
            upsert_parent_ids: UInt16Array::from(rows.iter().map(|(p, _)| *p).collect::<Vec<_>>()),
        })
        .collect::<Vec<_>>();
    // `upserts` can be empty when every annotation reused an already-present id
    // (e.g. all forwards of an already-seeded item) — then keep the batch as-is.
    let new_resource_attrs = if upserts.is_empty() {
        existing_ra.clone()
    } else {
        upsert_attributes::<UInt16Type>(existing_ra, &upserts)
            .map_err(|e| anyhow::anyhow!("upsert resource attrs: {e}"))?
    };

    // Rewrite the root record's resource.id to point at the assigned parent_ids.
    let Some(record) = get_record(pl) else {
        return Ok(ListenerAnnotate {
            for_forward: vec![],
            for_process: vec![
                (payload::ProcessorPayload {
                    processor_id: None,
                    payload: pl.clone(),
                }),
            ],
        });
    };
    let new_root = set_resource_ids(record, &row_parent_id)?;
    let root_type = match &pl.data {
        OtapArrowRecords::Logs(_) => ArrowPayloadType::Logs,
        OtapArrowRecords::Metrics(_) => ArrowPayloadType::UnivariateMetrics,
        OtapArrowRecords::Traces(_) => ArrowPayloadType::Spans,
    };
    let mut data = pl.data.clone();
    data.set(root_type, new_root)
        .map_err(|e| anyhow::anyhow!("set root record: {e}"))?;
    data.set(ArrowPayloadType::ResourceAttrs, new_resource_attrs)
        .map_err(|e| anyhow::anyhow!("set resource attrs: {e}"))?;

    // Split the annotated batch into the locally-processed payloads (one per
    // processor id, plus a `None` bucket) and one payload per remote owner node.
    let mut pool = IdBitmapPool::new();
    if conf.is_some_and(|conf| conf.disable_cluster_forwarding) || disable_forwarding {
        // Forwarding is off: would-be-forwarded records are processed locally too.
        return Ok(ListenerAnnotate {
            for_forward: vec![],
            for_process: split_for_process(&data, &row_dest, true, &mut pool)?,
        });
    }
    let forward_nodes: FxHashSet<Uuid> = row_dest
        .iter()
        .filter_map(|d| match d {
            Destination::Forward(node) => Some(*node),
            Destination::Process(_) => None,
        })
        .collect();
    let n = pl.num_rows();
    let mut for_forward = Vec::with_capacity(forward_nodes.len());
    for node in forward_nodes {
        let mask = BooleanArray::from(
            (0..n)
                .map(|i| row_dest[i] == Destination::Forward(node))
                .collect::<Vec<_>>(),
        );
        let sub = filter_otap_batch(&mask, &data, &mut pool)
            .map_err(|e| anyhow::anyhow!("filter for_forward: {e}"))?;
        for_forward.push(payload::ForwarderPayload {
            node_id: node,
            payload: payload::Payload::new(sub),
        });
    }
    let for_process = split_for_process(&data, &row_dest, false, &mut pool)?;
    Ok(ListenerAnnotate {
        for_forward,
        for_process,
    })
}

/// Split the annotated `data` into one payload per processor id. Each `Process` row
/// is emitted under every processor id it carries, or under `None` when it carries
/// none. With `include_forward` true (forwarding disabled) `Forward` rows are
/// processed locally under `None` too (they only carry patrolli.item.id, so the
/// processor resolves their item + processors by id); otherwise they are dropped
/// here because they are shipped out via `for_forward`.
fn split_for_process(
    data: &OtapArrowRecords,
    row_dest: &[Destination],
    include_forward: bool,
    pool: &mut IdBitmapPool,
) -> Result<Vec<payload::ProcessorPayload>> {
    let n = row_dest.len();
    let mut masks: FxHashMap<Option<Uuid>, Vec<bool>> = FxHashMap::default();
    for (i, dest) in row_dest.iter().enumerate() {
        match dest {
            Destination::Process(processors) if processors.is_empty() => {
                masks.entry(None).or_insert_with(|| vec![false; n])[i] = true;
            }
            Destination::Process(processors) => {
                for p in processors {
                    masks.entry(Some(*p)).or_insert_with(|| vec![false; n])[i] = true;
                }
            }
            Destination::Forward(_) if include_forward => {
                masks.entry(None).or_insert_with(|| vec![false; n])[i] = true;
            }
            Destination::Forward(_) => {}
        }
    }
    let mut out = Vec::with_capacity(masks.len());
    for (key, mask) in masks {
        let sub = filter_otap_batch(&BooleanArray::from(mask), data, pool)
            .map_err(|e| anyhow::anyhow!("filter for_process: {e}"))?;
        out.push(payload::ProcessorPayload {
            processor_id: key,
            payload: payload::Payload::new(sub),
        });
    }
    Ok(out)
}

/// Where an annotated record should go.
#[derive(Clone, PartialEq, Eq)]
enum Destination {
    Process(SmallVec<[Uuid; 4]>),
    Forward(Uuid),
}

/// Per-key (parent_id, value) rows to upsert into the output `ResourceAttrs`.
#[derive(Default)]
struct AttrCollectors {
    rows: FxHashMap<&'static str, Vec<(u16, String)>>,
}

impl AttrCollectors {
    fn push(&mut self, key: payload::ResourceAttrKey, parent_id: u16, value: String) {
        self.rows
            .entry(key.as_str())
            .or_default()
            .push((parent_id, value));
    }
}

/// Resolve `item_id` to a resource parent_id, reusing an existing one when present
/// (returns `true`) or allocating a fresh id otherwise (returns `false`).
fn alloc_parent_id(
    item_to_parent: &mut FxHashMap<Uuid, u16>,
    next_parent_id: &mut u16,
    item_id: Uuid,
) -> (u16, bool) {
    match item_to_parent.get(&item_id) {
        Some(pid) => (*pid, true),
        None => {
            let pid = *next_parent_id;
            *next_parent_id += 1;
            item_to_parent.insert(item_id, pid);
            (pid, false)
        }
    }
}

/// Push a full resource's attributes into `collectors` (key set mirrors
/// [`payload::append_resource`]). `include_item_id` is false when the record already
/// carries `patrolli.item.id` (a seeded record), so we don't duplicate it.
fn push_resource(
    collectors: &mut AttrCollectors,
    parent_id: u16,
    res: &payload::Resource,
    include_item_id: bool,
) {
    // TODO: can be further optimized
    use payload::ResourceAttrKey as K;
    let host_id = res.host_id.as_hyphenated().to_string();
    collectors.push(K::ServiceName, parent_id, res.host_name.to_string());
    collectors.push(K::ServiceInstanceId, parent_id, host_id.clone());
    collectors.push(K::HostName, parent_id, res.host_name.to_string());
    collectors.push(K::HostId, parent_id, host_id);
    collectors.push(K::ItemName, parent_id, res.item_name.to_string());
    if include_item_id {
        collectors.push(
            K::ItemId,
            parent_id,
            res.item_id.as_hyphenated().to_string(),
        );
    }
    if let Some(endpoint_id) = res.endpoint_id {
        collectors.push(
            K::EndpointId,
            parent_id,
            endpoint_id.as_hyphenated().to_string(),
        );
    }
    if let Some(endpoint_name) = &res.endpoint_name {
        collectors.push(K::EndpointName, parent_id, endpoint_name.to_string());
    }
    if let Some(endpoint_type) = &res.endpoint_type {
        collectors.push(K::EndpointType, parent_id, endpoint_type.to_string());
    }
    if let Some(host_timezone) = &res.host_timezone {
        collectors.push(K::HostTimezone, parent_id, host_timezone.to_string());
    }
}

/// Rebuild `record` with a new per-row `resource.id` (matched rows get `Some`,
/// unmatched get `None` so they reference no resource).
fn set_resource_ids(record: &RecordBatch, ids: &[Option<u16>]) -> Result<RecordBatch> {
    let res_idx = record
        .schema()
        .index_of(consts::RESOURCE)
        .map_err(|e| anyhow::anyhow!("record has no resource column: {e}"))?;
    let res_struct = record
        .column(res_idx)
        .as_struct_opt()
        .ok_or_else(|| anyhow::anyhow!("resource column is not a struct"))?;

    let (fields, mut child_arrays, nulls) = res_struct.clone().into_parts();
    let id_child = fields
        .iter()
        .position(|f| f.name() == consts::ID)
        .ok_or_else(|| anyhow::anyhow!("resource struct has no id field"))?;
    child_arrays[id_child] = Arc::new(UInt16Array::from(ids.to_vec())) as ArrayRef;
    let new_struct = StructArray::try_new(fields, child_arrays, nulls)
        .map_err(|e| anyhow::anyhow!("rebuild resource struct: {e}"))?;

    let mut columns: Vec<ArrayRef> = record.columns().to_vec();
    columns[res_idx] = Arc::new(new_struct);
    RecordBatch::try_new(record.schema(), columns)
        .map_err(|e| anyhow::anyhow!("rebuild record batch: {e}"))
}

fn monitored_by_us(id: Option<Uuid>, node_id: Option<Uuid>) -> bool {
    match (id, node_id) {
        (Some(id), Some(node_id)) => id == node_id,
        _ => true,
    }
}

fn attr_has_val(attr_batch: &RecordBatch, col: &str, val: &str) -> Result<Option<BooleanArray>> {
    if attr_batch.num_rows() == 0 {
        return Ok(None);
    };
    let Some(attr_keys_col) = attr_batch.column_by_name(col) else {
        return Ok(None);
    };
    let val = StringArray::new_scalar(val);
    Ok(Some(kernels::cmp::eq(&attr_keys_col, &val)?))
}

fn match_by_ip(pl: &payload::Payload, ip: IpAddr, set: &mut FxHashSet<u16>) -> Result<()> {
    let record_attr = get_record_attr(pl);
    let Some(record_attr) = record_attr else {
        return Ok(());
    };
    let Some(has_ip_field) = attr_has_val(
        record_attr,
        consts::ATTRIBUTE_KEY,
        payload::RecordAttrKey::SourceAddress.as_str(),
    )?
    else {
        return Ok(());
    };
    let mut ip_buf = arrayvec::ArrayString::<45>::new(); // 45 = max IPv6 string len
    write!(&mut ip_buf, "{}", ip)?;
    let Some(has_ip_value) = attr_has_val(record_attr, consts::ATTRIBUTE_STR, &ip_buf)? else {
        return Ok(());
    };
    let has_ip = kernels::boolean::and(&has_ip_field, &has_ip_value)?;
    collect_parent_ids(record_attr, &has_ip, set)
}

fn item_id_resource_build_map(pl: &payload::Payload) -> Result<ItemIdResourceMap> {
    let Some(resource_attr) = pl.data.get(ArrowPayloadType::ResourceAttrs) else {
        // don't have resource attributes mean all don't have item_id. listener other than
        // open_telemetry should use this path (naive listener doesn't push resource attributes)
        return Ok(ItemIdResourceMap::default());
    };
    if resource_attr.num_rows() == 0 {
        return Ok(ItemIdResourceMap::default());
    };
    let Some(attr_val_str_col) = resource_attr.column_by_name(consts::ATTRIBUTE_STR) else {
        return Ok(ItemIdResourceMap::default());
    };
    let attr_val_str_col = StrCol::try_new(attr_val_str_col.as_ref())?;
    let Some(attr_parent_id_col) = resource_attr.column_by_name(consts::PARENT_ID) else {
        return Ok(ItemIdResourceMap::default());
    };
    let attr_parent_id_col = attr_parent_id_col
        .as_primitive_opt::<UInt16Type>()
        .ok_or_else(|| anyhow::anyhow!("Failed to convert parent ID column to UInt16Type"))?;
    let Some(with_item_id_masks) = attr_has_val(
        resource_attr,
        consts::ATTRIBUTE_KEY,
        payload::ResourceAttrKey::ItemId.as_str(),
    )?
    else {
        return Ok(ItemIdResourceMap::default());
    };
    let all_false = kernels::aggregate::bool_or(&with_item_id_masks) != Some(true);
    if all_false {
        return Ok(ItemIdResourceMap::default());
    };
    let mut item_resource_map = ItemIdResourceMap::default();
    for (i, mask) in with_item_id_masks.iter().enumerate() {
        if mask == Some(true) && attr_val_str_col.is_valid(i) && attr_parent_id_col.is_valid(i) {
            let Ok(id) = Uuid::try_parse(attr_val_str_col.value(i)) else {
                continue;
            };
            let parent_id = attr_parent_id_col.value(i);
            item_resource_map.item_to_parent_id.insert(id, parent_id);
            item_resource_map.parent_id_to_item.insert(parent_id, id);
            item_resource_map.max_parent_id = item_resource_map.max_parent_id.max(parent_id);
        }
    }
    Ok(item_resource_map)
}

/// Pre-fill `item_ids` for records that already reference a resource carrying an
/// item_id. Each record points at a `resource.id`; `existing` maps that id back
/// to the item it was assigned to (built by [`item_id_resource_build_map`]).
fn seed_existing_item_ids(
    pl: &payload::Payload,
    existing: &ItemIdResourceMap,
    item_ids: &mut [Option<Annotate>],
) -> Result<()> {
    if existing.parent_id_to_item.is_empty() {
        return Ok(());
    }
    let Some(record) = get_record(pl) else {
        return Ok(());
    };
    let Some(res_id_col) = struct_id_column(record, consts::RESOURCE) else {
        return Ok(());
    };
    let res_id_col = res_id_col
        .as_primitive_opt::<UInt16Type>()
        .ok_or_else(|| anyhow::anyhow!("Failed to convert resource id column to UInt16Type"))?;
    for (i, res_id) in res_id_col.iter().enumerate() {
        if let Some(res_id) = res_id
            && let Some(item_id) = existing.parent_id_to_item.get(&res_id)
        {
            item_ids[i] = Some(Annotate::ItemId(*item_id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Annotate, Destination, alloc_parent_id, attr_has_val, item_id_resource_build_map,
        item_listener_annotate, match_by_ip, monitored_by_us, seed_existing_item_ids,
        set_resource_ids, split_for_process,
    };
    use crate::config::item::ListenerItemMatchStrategy;
    use crate::payload::attr::{get_record, get_record_attr, parent_id_values, struct_id_column};
    use crate::{cache, config, payload};
    use arrow::array::{Array, AsArray, RecordBatch};
    use arrow::datatypes::UInt16Type;
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::otap::filter::IdBitmapPool;
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, KeyValue};
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource as OtlpResource;
    use otel_arrow_dfe_pdata::schema::consts;
    use rustc_hash::{FxHashMap, FxHashSet};
    use smallvec::smallvec;
    use std::net::IpAddr;
    use std::time::Duration;
    use uuid::Uuid;

    // --- payload fixtures (mirror the payload::attr test module) ---

    fn kv(key: &str, value: Value) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue { value: Some(value) }),
        }
    }

    fn sval(s: &str) -> Value {
        Value::StringValue(s.to_string())
    }

    fn record(attrs: Vec<KeyValue>) -> LogRecord {
        LogRecord {
            time_unix_nano: 1,
            attributes: attrs,
            ..Default::default()
        }
    }

    /// A record carrying a `source.address` attribute (drives `match_by_ip` and the
    /// IP-based match strategies).
    fn ip_record(ip: &str) -> LogRecord {
        record(vec![kv(
            payload::RecordAttrKey::SourceAddress.as_str(),
            sval(ip),
        )])
    }

    fn logs_payload(records: Vec<LogRecord>) -> payload::Payload {
        let req = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: records,
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        payload::Payload::new(encode_logs_otap_batch(&req).unwrap())
    }

    fn logs_payload_with_resource(
        res_attrs: Vec<KeyValue>,
        records: Vec<LogRecord>,
    ) -> payload::Payload {
        let req = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: Some(OtlpResource {
                    attributes: res_attrs,
                    ..Default::default()
                }),
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: records,
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        payload::Payload::new(encode_logs_otap_batch(&req).unwrap())
    }

    // --- cache / config fixtures (in-memory, no DB) ---

    fn listener_conf(id: Uuid, disable_cluster_forwarding: bool) -> config::listener::Listener {
        config::listener::Listener::new(
            id,
            "test-listener".into(),
            1,
            config::Batch::default(),
            config::listener::Config::Dummy(config::listener::Dummy {
                speed: 5,
                messages_per_batch: 100,
            }),
            disable_cluster_forwarding,
        )
    }

    struct TestEnv {
        cache: cache::Cache,
        listener_conf: config::listener::Listener,
        item_id: Uuid,
        processor_id: Uuid,
    }

    /// Build a cache with one host, one endpoint, and a single listener item using
    /// `match_strategy`, owned by `monitored_by`. `endpoint_ip` registers the
    /// endpoint's source IP (needed by `EndpointSourceIp`).
    fn build_env(
        match_strategy: ListenerItemMatchStrategy,
        monitored_by: Option<Uuid>,
        endpoint_ip: Option<IpAddr>,
        disable_cluster_forwarding: bool,
    ) -> TestEnv {
        let cache = cache::Cache::new();
        let host_id = Uuid::now_v7();
        let endpoint_id = Uuid::now_v7();
        let listener_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let processor_id = Uuid::now_v7();

        cache.host.write().insert(config::host::Host {
            id: host_id,
            name: "test-host".into(),
            timezone: None,
        });
        cache
            .endpoint
            .write()
            .insert(config::endpoint::Endpoint::new(
                endpoint_id,
                "test-endpoint".into(),
                host_id,
                config::endpoint::Config::SnmpV1(config::endpoint::SnmpV1 {
                    address: "127.0.0.1:161".parse().unwrap(),
                    community: "public".to_string(),
                }),
            ));
        let item = config::item::Item::new(
            item_id,
            "test-item".into(),
            Some(endpoint_id),
            host_id,
            config::item::Schedule::Never,
            Some(listener_id),
            Duration::from_secs(1),
            vec![processor_id],
            config::item::Config::Listener(config::item::Listener { match_strategy }),
        );
        cache.item.write().insert(item.clone());
        {
            let mut lis = cache.listener.write();
            lis.insert_item(item, monitored_by);
            if let Some(ip) = endpoint_ip {
                lis.insert_ip(endpoint_id, ip);
            }
        }
        TestEnv {
            cache,
            listener_conf: listener_conf(listener_id, disable_cluster_forwarding),
            item_id,
            processor_id,
        }
    }

    /// Total records routed to `for_process` entries matching `processor_id`.
    fn process_len(res: &super::ListenerAnnotate, processor_id: Option<Uuid>) -> usize {
        res.for_process
            .iter()
            .filter(|p| p.processor_id == processor_id)
            .map(|p| p.payload.num_items())
            .sum()
    }

    /// Number of rows in a `ResourceAttrs` batch whose attribute key equals `key`.
    fn count_attr_key_rows(ra: &RecordBatch, key: &str) -> usize {
        attr_has_val(ra, consts::ATTRIBUTE_KEY, key)
            .unwrap()
            .map(|m| m.iter().filter(|x| *x == Some(true)).count())
            .unwrap_or(0)
    }

    // --- pure helpers ---

    #[test]
    fn monitored_by_us_logic() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        assert!(monitored_by_us(None, Some(a)));
        assert!(monitored_by_us(Some(a), None));
        assert!(monitored_by_us(None, None));
        assert!(monitored_by_us(Some(a), Some(a)));
        assert!(!monitored_by_us(Some(a), Some(b)));
    }

    #[test]
    fn alloc_parent_id_allocates_and_reuses() {
        let mut map = FxHashMap::default();
        let mut next = 0u16;
        let item_a = Uuid::now_v7();
        let item_b = Uuid::now_v7();
        assert_eq!(alloc_parent_id(&mut map, &mut next, item_a), (0, false));
        assert_eq!(next, 1);
        // same item → reuse the existing parent id, no bump
        assert_eq!(alloc_parent_id(&mut map, &mut next, item_a), (0, true));
        assert_eq!(next, 1);
        // new item → fresh id
        assert_eq!(alloc_parent_id(&mut map, &mut next, item_b), (1, false));
        assert_eq!(next, 2);
    }

    #[test]
    fn attr_has_val_matches_key() {
        let pl = logs_payload(vec![record(vec![kv("k", sval("v"))])]);
        let ra = get_record_attr(&pl).expect("record attrs");
        let present = attr_has_val(ra, consts::ATTRIBUTE_KEY, "k")
            .unwrap()
            .unwrap();
        assert!(present.iter().any(|x| x == Some(true)));
        let absent = attr_has_val(ra, consts::ATTRIBUTE_KEY, "nope")
            .unwrap()
            .unwrap();
        assert!(absent.iter().all(|x| x != Some(true)));
    }

    #[test]
    fn match_by_ip_collects_matching_parent_ids() {
        let pl = logs_payload(vec![ip_record("10.0.0.1"), ip_record("10.0.0.2")]);
        let mut set = FxHashSet::default();
        match_by_ip(&pl, "10.0.0.1".parse().unwrap(), &mut set).unwrap();
        assert_eq!(set.len(), 1);
        assert!(set.contains(&0)); // record 0's log id

        let mut empty = FxHashSet::default();
        match_by_ip(&pl, "192.168.0.1".parse().unwrap(), &mut empty).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn item_id_resource_build_map_reads_item_id() {
        let item_id = Uuid::now_v7();
        let pl = logs_payload_with_resource(
            vec![kv(
                payload::ResourceAttrKey::ItemId.as_str(),
                sval(&item_id.as_hyphenated().to_string()),
            )],
            vec![record(vec![])],
        );
        let map = item_id_resource_build_map(&pl).unwrap();
        assert_eq!(map.item_to_parent_id.get(&item_id), Some(&0));
        assert_eq!(map.parent_id_to_item.get(&0), Some(&item_id));

        // no resource attrs → empty map
        let pl2 = logs_payload(vec![record(vec![])]);
        assert!(
            item_id_resource_build_map(&pl2)
                .unwrap()
                .item_to_parent_id
                .is_empty()
        );
    }

    #[test]
    fn seed_existing_item_ids_seeds_referencing_records() {
        let item_id = Uuid::now_v7();
        let pl = logs_payload_with_resource(
            vec![kv(
                payload::ResourceAttrKey::ItemId.as_str(),
                sval(&item_id.as_hyphenated().to_string()),
            )],
            vec![record(vec![])],
        );
        let existing = item_id_resource_build_map(&pl).unwrap();
        let mut item_ids: Vec<Option<Annotate>> = vec![None; pl.num_rows()];
        seed_existing_item_ids(&pl, &existing, &mut item_ids).unwrap();
        assert!(matches!(item_ids[0], Some(Annotate::ItemId(id)) if id == item_id));
    }

    #[test]
    fn set_resource_ids_rewrites_resource_id_column() {
        let pl = logs_payload_with_resource(
            vec![kv("host.name", sval("h"))],
            vec![record(vec![]), record(vec![])],
        );
        let root = get_record(&pl).expect("root record");
        let new = set_resource_ids(root, &[Some(7), None]).unwrap();
        let id_col = struct_id_column(&new, consts::RESOURCE).expect("resource id col");
        let id_col = id_col.as_primitive::<UInt16Type>();
        assert_eq!(id_col.value(0), 7);
        assert!(id_col.is_null(1));
    }

    #[test]
    fn split_for_process_groups_by_processor_and_handles_forward() {
        let pl = logs_payload(vec![
            record(vec![kv("k", sval("a"))]),
            record(vec![kv("k", sval("b"))]),
            record(vec![kv("k", sval("c"))]),
        ]);
        let processor = Uuid::now_v7();
        let node = Uuid::now_v7();
        let row_dest = vec![
            Destination::Process(smallvec![processor]),
            Destination::Process(smallvec![]),
            Destination::Forward(node),
        ];
        let mut pool = IdBitmapPool::new();

        // include_forward = false → the Forward row is dropped.
        let out = split_for_process(&pl.data, &row_dest, false, &mut pool).unwrap();
        let by_processor: usize = out
            .iter()
            .filter(|p| p.processor_id == Some(processor))
            .map(|p| p.payload.num_items())
            .sum();
        let by_none: usize = out
            .iter()
            .filter(|p| p.processor_id.is_none())
            .map(|p| p.payload.num_items())
            .sum();
        assert_eq!(by_processor, 1);
        assert_eq!(by_none, 1); // only the Process([]) row

        // include_forward = true → the Forward row joins the None bucket.
        let out2 = split_for_process(&pl.data, &row_dest, true, &mut pool).unwrap();
        let by_none2: usize = out2
            .iter()
            .filter(|p| p.processor_id.is_none())
            .map(|p| p.payload.num_items())
            .sum();
        assert_eq!(by_none2, 2); // Process([]) + Forward
    }

    // --- item_listener_annotate (integration) ---

    #[test]
    fn annotate_empty_payload_passes_through() {
        let env = build_env(ListenerItemMatchStrategy::None, None, None, false);
        let pl = logs_payload(vec![]);
        let res =
            item_listener_annotate(&pl, Some(&env.listener_conf), &env.cache, None, false).unwrap();
        assert!(res.for_forward.is_empty());
        assert_eq!(res.for_process.len(), 1);
        assert_eq!(res.for_process[0].processor_id, None);
        assert_eq!(res.for_process[0].payload.num_items(), 0);
    }

    #[test]
    fn annotate_listener_without_items_passes_through() {
        let cache = cache::Cache::new();
        let conf = listener_conf(Uuid::now_v7(), false);
        let pl = logs_payload(vec![record(vec![kv("k", sval("v"))])]);
        let res = item_listener_annotate(&pl, Some(&conf), &cache, None, false).unwrap();
        assert!(res.for_forward.is_empty());
        assert_eq!(res.for_process.len(), 1);
        assert_eq!(res.for_process[0].processor_id, None);
        assert_eq!(res.for_process[0].payload.num_items(), 1);
    }

    #[test]
    fn annotate_match_all_routes_to_processor_and_annotates() {
        let env = build_env(ListenerItemMatchStrategy::None, None, None, false);
        let pl = logs_payload(vec![
            record(vec![kv("k", sval("v"))]),
            record(vec![kv("k", sval("w"))]),
        ]);
        let res =
            item_listener_annotate(&pl, Some(&env.listener_conf), &env.cache, None, false).unwrap();
        assert!(res.for_forward.is_empty());
        assert_eq!(res.for_process.len(), 1);
        let pp = &res.for_process[0];
        assert_eq!(pp.processor_id, Some(env.processor_id));
        assert_eq!(pp.payload.num_items(), 2);

        // resource attributes were upserted onto the annotated payload.
        let ra = pp
            .payload
            .data
            .get(ArrowPayloadType::ResourceAttrs)
            .expect("resource attrs");
        let host = parent_id_values(ra, "host.name").unwrap();
        assert!(host.values().any(|v| v.as_str() == "test-host"));
        let want_item = env.item_id.as_hyphenated().to_string();
        let item = parent_id_values(ra, payload::ResourceAttrKey::ItemId.as_str()).unwrap();
        assert!(item.values().any(|v| v.as_str() == want_item.as_str()));
    }

    #[test]
    fn annotate_does_not_duplicate_resource_attrs_across_records() {
        use payload::ResourceAttrKey as K;
        // Match-all so every record resolves to the SAME item, and thus shares one
        // resource parent_id. Each resource attribute must be emitted once for that
        // shared resource — not once per record. Regression test for the bug where
        // every key except patrolli.item.id was duplicated per record (so the OTLP
        // denormalized output repeated host/service attributes thousands of times).
        let env = build_env(ListenerItemMatchStrategy::None, None, None, false);
        let pl = logs_payload(vec![
            record(vec![kv("k", sval("a"))]),
            record(vec![kv("k", sval("b"))]),
            record(vec![kv("k", sval("c"))]),
            record(vec![kv("k", sval("d"))]),
            record(vec![kv("k", sval("e"))]),
        ]);
        let res =
            item_listener_annotate(&pl, Some(&env.listener_conf), &env.cache, None, false).unwrap();

        let pp = res
            .for_process
            .iter()
            .find(|p| p.processor_id == Some(env.processor_id))
            .expect("processor payload");
        assert_eq!(pp.payload.num_items(), 5);

        let ra = pp
            .payload
            .data
            .get(ArrowPayloadType::ResourceAttrs)
            .expect("resource attrs");

        for key in [
            K::ServiceName,
            K::ServiceInstanceId,
            K::HostName,
            K::HostId,
            K::ItemName,
            K::ItemId,
        ] {
            assert_eq!(
                count_attr_key_rows(ra, key.as_str()),
                1,
                "resource attr {:?} should appear exactly once, not once per record",
                key.as_str(),
            );
        }
    }

    #[test]
    fn annotate_source_ips_matches_only_listed_ip() {
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let env = build_env(
            ListenerItemMatchStrategy::SourceIps(vec![ip]),
            None,
            None,
            false,
        );
        let pl = logs_payload(vec![ip_record("10.0.0.1"), ip_record("10.0.0.2")]);
        let res =
            item_listener_annotate(&pl, Some(&env.listener_conf), &env.cache, None, false).unwrap();
        assert!(res.for_forward.is_empty());
        assert_eq!(process_len(&res, Some(env.processor_id)), 1);
        assert_eq!(process_len(&res, None), 1); // unmatched record
    }

    #[test]
    fn annotate_endpoint_source_ip_matches_registered_ip() {
        let ip: IpAddr = "10.0.0.5".parse().unwrap();
        let env = build_env(
            ListenerItemMatchStrategy::EndpointSourceIp,
            None,
            Some(ip),
            false,
        );
        let pl = logs_payload(vec![ip_record("10.0.0.5"), ip_record("10.0.0.6")]);
        let res =
            item_listener_annotate(&pl, Some(&env.listener_conf), &env.cache, None, false).unwrap();
        assert_eq!(process_len(&res, Some(env.processor_id)), 1);
        assert_eq!(process_len(&res, None), 1);
    }

    #[test]
    fn annotate_attribute_match_matches_by_attr() {
        use crate::config::processor::{
            AttributeFilter, AttributeFilterExpr, AttributeTable, KeyValueOp, Value as CfgValue,
        };
        let filter = AttributeFilter::Match(AttributeFilterExpr {
            table: AttributeTable::Record,
            key: "env".to_string(),
            key_is_regex: false,
            op: KeyValueOp::Equal,
            value: CfgValue::String("prod".into()),
        });
        let env = build_env(
            ListenerItemMatchStrategy::AttributeMatch(filter),
            None,
            None,
            false,
        );
        let pl = logs_payload(vec![
            record(vec![kv("env", sval("prod"))]),
            record(vec![kv("env", sval("dev"))]),
        ]);
        let res =
            item_listener_annotate(&pl, Some(&env.listener_conf), &env.cache, None, false).unwrap();
        assert_eq!(process_len(&res, Some(env.processor_id)), 1);
        assert_eq!(process_len(&res, None), 1);
    }

    #[test]
    fn annotate_forwards_records_owned_by_remote_node() {
        let other_node = Uuid::now_v7();
        let our_node = Uuid::now_v7();
        let env = build_env(
            ListenerItemMatchStrategy::None,
            Some(other_node),
            None,
            false,
        );
        let pl = logs_payload(vec![record(vec![kv("k", sval("v"))])]);
        let res = item_listener_annotate(
            &pl,
            Some(&env.listener_conf),
            &env.cache,
            Some(our_node),
            false,
        )
        .unwrap();
        assert_eq!(res.for_forward.len(), 1);
        assert_eq!(res.for_forward[0].node_id, other_node);
        assert_eq!(res.for_forward[0].payload.num_items(), 1);
        // nothing processed locally
        let local: usize = res.for_process.iter().map(|p| p.payload.num_items()).sum();
        assert_eq!(local, 0);
    }

    #[test]
    fn annotate_forwarding_disabled_processes_locally() {
        let other_node = Uuid::now_v7();
        let our_node = Uuid::now_v7();
        let env = build_env(
            ListenerItemMatchStrategy::None,
            Some(other_node),
            None,
            false,
        );
        let pl = logs_payload(vec![record(vec![kv("k", sval("v"))])]);
        // disable_forwarding = true → would-be-forwarded records run locally.
        let res = item_listener_annotate(
            &pl,
            Some(&env.listener_conf),
            &env.cache,
            Some(our_node),
            true,
        )
        .unwrap();
        assert!(res.for_forward.is_empty());
        let local: usize = res.for_process.iter().map(|p| p.payload.num_items()).sum();
        assert_eq!(local, 1);
    }

    #[test]
    fn annotate_disable_cluster_forwarding_processes_locally() {
        let other_node = Uuid::now_v7();
        let our_node = Uuid::now_v7();
        // listener config has disable_cluster_forwarding = true
        let env = build_env(
            ListenerItemMatchStrategy::None,
            Some(other_node),
            None,
            true,
        );
        let pl = logs_payload(vec![record(vec![kv("k", sval("v"))])]);
        let res = item_listener_annotate(
            &pl,
            Some(&env.listener_conf),
            &env.cache,
            Some(our_node),
            false,
        )
        .unwrap();
        assert!(res.for_forward.is_empty());
        let local: usize = res.for_process.iter().map(|p| p.payload.num_items()).sum();
        assert_eq!(local, 1);
    }

    #[test]
    fn annotate_seeded_item_id_resolves_to_processor() {
        let env = build_env(ListenerItemMatchStrategy::None, None, None, false);
        // payload already carries this item's id in resource attrs → seeded path.
        let pl = logs_payload_with_resource(
            vec![kv(
                payload::ResourceAttrKey::ItemId.as_str(),
                sval(&env.item_id.as_hyphenated().to_string()),
            )],
            vec![record(vec![])],
        );
        let res =
            item_listener_annotate(&pl, Some(&env.listener_conf), &env.cache, None, false).unwrap();
        assert!(res.for_forward.is_empty());
        assert_eq!(process_len(&res, Some(env.processor_id)), 1);
    }

    #[test]
    fn annotate_seeded_item_id_passes_through_when_unresolvable() {
        let cache = cache::Cache::new();
        let conf = listener_conf(Uuid::now_v7(), false);
        let item_id = Uuid::now_v7();
        // resource carries an item id the cache can't resolve → pure passthrough.
        let pl = logs_payload_with_resource(
            vec![kv(
                payload::ResourceAttrKey::ItemId.as_str(),
                sval(&item_id.as_hyphenated().to_string()),
            )],
            vec![record(vec![])],
        );
        let res = item_listener_annotate(&pl, Some(&conf), &cache, None, false).unwrap();
        assert!(res.for_forward.is_empty());
        assert_eq!(res.for_process.len(), 1);
        assert_eq!(res.for_process[0].processor_id, None);
        assert_eq!(res.for_process[0].payload.num_items(), 1);
    }

    // --- per-item stats ---

    fn snapshot(env: &TestEnv) -> cache::item::StatsSnapshot {
        env.cache
            .item
            .read()
            .stats(&env.item_id)
            .expect("item in cache")
            .snapshot()
    }

    #[test]
    fn ingest_is_recorded_for_an_item_this_node_owns() {
        let our_node = Uuid::now_v7();
        let env = build_env(ListenerItemMatchStrategy::None, Some(our_node), None, false);
        assert_eq!(snapshot(&env).state, cache::item::ItemState::NeverRan);

        let pl = logs_payload(vec![record(vec![]), record(vec![]), record(vec![])]);
        item_listener_annotate(
            &pl,
            Some(&env.listener_conf),
            &env.cache,
            Some(our_node),
            false,
        )
        .unwrap();

        let s = snapshot(&env);
        assert_eq!(s.state, cache::item::ItemState::Ok);
        // one receive event, three rows
        assert_eq!(s.total_checks, 1);
        assert_eq!(s.total_records, 3);
        assert_eq!(s.last_record_count, Some(3));
        assert!(s.last_success.is_some());
        assert!(s.last_failure.is_none());
        // no poll cycle: nothing to be late for, nothing to retry
        assert_eq!(s.last_lag_ms, Some(0));
        assert_eq!(s.retries, 0);
        assert_eq!(s.next_check, None);
        // but the arrival does light its bucket in the windowed ring
        let ok: u32 = s.activity_ok.iter().map(|b| b.count_ones()).sum();
        assert_eq!(ok, 1);
    }

    #[test]
    fn ingest_accumulates_across_batches() {
        let env = build_env(ListenerItemMatchStrategy::None, None, None, false);
        for _ in 0..4 {
            let pl = logs_payload(vec![record(vec![]), record(vec![])]);
            item_listener_annotate(&pl, Some(&env.listener_conf), &env.cache, None, false).unwrap();
        }
        let s = snapshot(&env);
        assert_eq!(s.total_checks, 4);
        assert_eq!(s.total_records, 8);
        assert_eq!(s.last_record_count, Some(2));
    }

    /// The core of the "stats live on the owning node" model: the node that merely
    /// receives the traffic records nothing, so no two nodes ever hold counters for
    /// the same item and the stats API never has to merge them.
    #[test]
    fn ingest_is_not_recorded_for_an_item_owned_by_another_node() {
        let other_node = Uuid::now_v7();
        let our_node = Uuid::now_v7();
        let env = build_env(
            ListenerItemMatchStrategy::None,
            Some(other_node),
            None,
            false,
        );
        let pl = logs_payload(vec![record(vec![]), record(vec![])]);
        let res = item_listener_annotate(
            &pl,
            Some(&env.listener_conf),
            &env.cache,
            Some(our_node),
            false,
        )
        .unwrap();

        assert_eq!(res.for_forward.len(), 1);
        let s = snapshot(&env);
        assert_eq!(s.state, cache::item::ItemState::NeverRan);
        assert_eq!(s.total_checks, 0);
        assert_eq!(s.total_records, 0);
    }

    /// The other half: the owner node re-annotates the forwarded copy (`conf: None`,
    /// forwarding disabled) via the seeded item-id path, and records it there. The
    /// row count survives the hop, so a forwarded item's counters are complete.
    #[test]
    fn the_owner_node_records_ingest_for_a_forwarded_payload() {
        let env = build_env(ListenerItemMatchStrategy::None, None, None, false);
        // What `ForwardPayloads` receives: rows carrying only patrolli.item.id.
        let pl = logs_payload_with_resource(
            vec![kv(
                payload::ResourceAttrKey::ItemId.as_str(),
                sval(&env.item_id.as_hyphenated().to_string()),
            )],
            vec![record(vec![]), record(vec![])],
        );
        // Exactly how `comm::forward_payloads` calls it.
        let res = item_listener_annotate(&pl, None, &env.cache, None, true).unwrap();

        assert_eq!(process_len(&res, Some(env.processor_id)), 2);
        let s = snapshot(&env);
        assert_eq!(s.state, cache::item::ItemState::Ok);
        assert_eq!(s.total_checks, 1);
        assert_eq!(s.total_records, 2);
    }

    #[test]
    fn unmatched_records_record_nothing() {
        let env = build_env(
            ListenerItemMatchStrategy::SourceIps(vec!["10.0.0.1".parse().unwrap()]),
            None,
            None,
            false,
        );
        // source.address doesn't match the item's strategy → no item resolved.
        let pl = logs_payload(vec![ip_record("192.0.2.7")]);
        item_listener_annotate(&pl, Some(&env.listener_conf), &env.cache, None, false).unwrap();

        let s = snapshot(&env);
        assert_eq!(s.state, cache::item::ItemState::NeverRan);
        assert_eq!(s.total_checks, 0);
    }

    /// A listener item whose host has no `monitored_by` isn't in any node's item
    /// cache, so there is no `Stats` to write. Annotation must still succeed.
    #[test]
    fn ingest_is_skipped_when_the_item_has_no_stats_entry() {
        let env = build_env(ListenerItemMatchStrategy::None, None, None, false);
        env.cache.item.write().remove(env.item_id);

        let pl = logs_payload(vec![record(vec![])]);
        let res =
            item_listener_annotate(&pl, Some(&env.listener_conf), &env.cache, None, false).unwrap();

        // still routed to the item's processor — the listener cache resolved it
        assert_eq!(process_len(&res, Some(env.processor_id)), 1);
        assert!(env.cache.item.read().stats(&env.item_id).is_none());
    }
}
