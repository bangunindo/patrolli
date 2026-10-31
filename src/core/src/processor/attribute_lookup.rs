// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The `attribute_lookup` processor: batched attribute enrichment from the
//! configured lookup tables. Config contract on
//! [`crate::config::processor::attribute_lookup`]; this module is its execution.
//!
//! Every rule reads the *incoming* attributes — one rule never sees another's
//! output — and the writes are applied together at the end, one rebuild per
//! affected attribute table.
//!
//! Three things are deliberately not errors, because none should cost
//! telemetry: an unbuildable probe key (counted by
//! [`TableRead::note_key_error`]), a probe that matches nothing, and a matched
//! row whose cell is null. Each leaves its destination untouched.
//!
//! The table is resolved from the registry once per batch, never cached on the
//! config: a config edit swaps the `Table` out and a CSV reload can change the
//! schema under a running processor, so a captured handle or `ColumnIdx` goes
//! stale. Resolution is O(configured columns), not O(records).

use super::attr_input;
use super::attr_input::Parent;
use crate::cache;
use crate::cache::lookup_table::TableRead;
use crate::cache::lookup_table::cell::{CellRef, KeyRef, Lookup, RangePoint};
use crate::cache::lookup_table::schema::{ColumnIdx, KeyPlan};
use crate::config::lookup_table as lt;
use crate::config::processor::AttributeTable;
use crate::config::processor::attribute_lookup::{
    AttributeLookup, AttributeLookupCast, AttributeLookupInput, AttributeLookupRule,
};
use crate::payload::{self, Payload, Value, attr};
use anyhow::{Result, anyhow, bail};
use arrow::array::{ArrayRef, AsArray, BooleanArray, RecordBatch, UInt16Array};
use arrow::datatypes::{DataType, Field, Schema, UInt16Type};
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::otap::transform::upsert_attributes::EMPTY_U16_ATTRS_RECORD_BATCH;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::consts;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use smol_str::{SmolStr, format_smolstr};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{trace, warn};
use uuid::Uuid;

/// Enrich `pl` in place and report whether any rule was skipped by an error.
/// No failing return: the payload always comes back, enriched as far as it got.
pub(super) fn attribute_lookup(
    mut pl: payload::Payload,
    config: &AttributeLookup,
    node_id: Uuid,
    cache: &cache::Cache,
) -> (payload::Payload, bool) {
    if pl.is_empty() {
        return (pl, false);
    }
    let mut errored = false;

    // Evaluated once and shared by every rule.
    let shared = match &config.when {
        None => None,
        Some(filter) => match attr::attribute_filter_mask(&pl, filter) {
            Ok(mask) => Some(mask),
            Err(e) => {
                warn!(error = %format!("{e:#}"), "shared `when` failed, skipping enrichment");
                return (pl, true);
            }
        },
    };

    let mut writes = Writes::default();
    for (i, rule) in config.lookups.iter().enumerate() {
        if let Err(e) = run_rule(&pl, rule, shared.as_ref(), node_id, cache, &mut writes) {
            errored = true;
            warn!(
                error = %format!("{e:#}"),
                lookup = i,
                lookup_table_id = %rule.lookup_table,
                "lookup skipped"
            );
        }
    }

    if let Err(e) = apply(&mut pl, writes, config.overwrite) {
        errored = true;
        warn!(error = %format!("{e:#}"), "writing enriched attributes failed");
    }
    (pl, errored)
}

// ---- collected writes ----

/// The values destined for one attribute key. One destination is fed by one
/// column, which has one declared type, so each map stores its native type.
///
/// `SmolStr`, not `String`: this is the one owned copy on the per-record path,
/// and what enrichment carries — a site, a region, an ASN — almost always fits
/// the 23-byte inline buffer, so it costs no allocation. Borrowing instead is
/// not an option: a mutable table's `Arc<MutableRow>` is dropped at the end of
/// the iteration, while these values must survive until `apply` merges them.
enum Values {
    Str(FxHashMap<Parent, SmolStr>),
    Int(FxHashMap<Parent, i64>),
    Double(FxHashMap<Parent, f64>),
    Bool(FxHashMap<Parent, bool>),
}

impl Values {
    fn new(ty: lt::ValueType) -> Self {
        match ty {
            // No OTLP/OTAP IP value, so an IP column is emitted as a string —
            // the same bridge `CellRef::to_value` makes.
            lt::ValueType::String | lt::ValueType::Ip => Values::Str(FxHashMap::default()),
            lt::ValueType::Integer => Values::Int(FxHashMap::default()),
            lt::ValueType::Double => Values::Double(FxHashMap::default()),
            lt::ValueType::Bool => Values::Bool(FxHashMap::default()),
        }
    }

    /// Records a non-null cell against `parent`. A cell disagreeing with its
    /// column's declared type is dropped rather than coerced; the typed column
    /// store makes that unreachable in practice.
    fn insert(&mut self, parent: Parent, cell: CellRef<'_>) {
        match (self, cell) {
            (Values::Str(m), CellRef::String(v)) => drop(m.insert(parent, SmolStr::new(v))),
            (Values::Str(m), CellRef::Ip(v)) => drop(m.insert(parent, format_smolstr!("{v}"))),
            (Values::Int(m), CellRef::Integer(v)) => drop(m.insert(parent, v)),
            (Values::Double(m), CellRef::Double(v)) => drop(m.insert(parent, v)),
            (Values::Bool(m), CellRef::Bool(v)) => drop(m.insert(parent, v)),
            _ => {}
        }
    }

    fn cell(&self, parent: Parent) -> CellRef<'_> {
        match self {
            Self::Str(m) => CellRef::String(&m[&parent]),
            Self::Int(m) => CellRef::Integer(m[&parent]),
            Self::Double(m) => CellRef::Double(m[&parent]),
            Self::Bool(m) => CellRef::Bool(m[&parent]),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Str(m) => m.len(),
            Self::Int(m) => m.len(),
            Self::Double(m) => m.len(),
            Self::Bool(m) => m.len(),
        }
    }

    fn for_each_parent(&self, visit: impl FnMut(Parent)) {
        match self {
            Self::Str(m) => m.keys().copied().for_each(visit),
            Self::Int(m) => m.keys().copied().for_each(visit),
            Self::Double(m) => m.keys().copied().for_each(visit),
            Self::Bool(m) => m.keys().copied().for_each(visit),
        }
    }
}

/// `attribute key -> values`. `BTreeMap` so the rebuilt batch does not depend on
/// hash iteration order.
type TableWrites = BTreeMap<SmolStr, Values>;

#[derive(Default)]
struct Writes {
    record: TableWrites,
    scope: TableWrites,
    resource: TableWrites,
}

impl Writes {
    fn table(&mut self, table: &AttributeTable) -> &mut TableWrites {
        match table {
            AttributeTable::Record => &mut self.record,
            AttributeTable::Scope => &mut self.scope,
            AttributeTable::Resource => &mut self.resource,
        }
    }
}

// ---- one rule ----

/// A rule's config resolved against one generation of its lookup table.
struct Plan<'a> {
    /// In the table's own key order, which is what `Lookup` expects — the config
    /// binds them by name, so YAML map order is irrelevant.
    keys: Vec<&'a AttributeLookupInput>,
    /// `Some` exactly when the table is range-keyed.
    point: Option<(&'a AttributeLookupInput, lt::RangeType)>,
    /// Destination keys are owned once here rather than per hit: `Writes` keys
    /// on `SmolStr`, and a long destination would otherwise allocate per record.
    outputs: Vec<(SmolStr, ColumnIdx, lt::ValueType)>,
}

impl<'a> Plan<'a> {
    fn resolve(read: &TableRead, rule: &'a AttributeLookupRule) -> Result<Self> {
        let schema = read.schema();
        // Range bound columns are consumed by the index and never materialized,
        // so they are not in `schema` and cannot be bound here.
        let bound = |idx: ColumnIdx| -> Result<&'a AttributeLookupInput> {
            let name = schema
                .name(idx)
                .ok_or_else(|| anyhow!("key column {idx} is missing from the table schema"))?;
            let ty = schema
                .value_type(idx)
                .ok_or_else(|| anyhow!("key column {name:?} has no type"))?;
            let input = rule
                .match_by
                .keys
                .get(name)
                .ok_or_else(|| anyhow!("match.keys has no binding for key column {name:?}"))?;
            // The index compares variants, so a wrong-typed binding misses on
            // every record. Fail the plan once instead of counting a miss per
            // record. In `bound`, not in one arm: a range table's partition
            // columns are bound the same way.
            match (input.cast, ty) {
                (Some(cast), ty) if cast_type(cast) != ty => {
                    bail!("match.keys[{name:?}].cast is {cast:?} but the column is {ty:?}")
                }
                // No OTAP IP value, so an ip key always arrives as a string.
                (None, lt::ValueType::Ip) => {
                    bail!("an ip key column needs an explicit `cast: ip` on match.keys[{name:?}]")
                }
                // `KeyRef` has no double variant. Config validation rejects a
                // double key column, but a mis-cast CSV column reaches here.
                (_, lt::ValueType::Double) => {
                    bail!("key column {name:?} is a double and cannot index a lookup table")
                }
                _ => {}
            }
            Ok(input)
        };
        let configured = rule.match_by.keys.len();
        let (keys, point) = match read.key_plan() {
            KeyPlan::Hash { columns } => {
                if rule.match_by.point.is_some() {
                    bail!("match.point is set but the lookup table is hash-keyed");
                }
                if configured != columns.len() {
                    bail!(
                        "match.keys binds {configured} column(s) but the table's hash key has {}",
                        columns.len()
                    );
                }
                (
                    columns.iter().map(|c| bound(*c)).collect::<Result<_>>()?,
                    None,
                )
            }
            KeyPlan::Range {
                partition,
                range_type,
            } => {
                let point = rule.match_by.point.as_ref().ok_or_else(|| {
                    anyhow!("the lookup table is range-keyed but match.point is not set")
                })?;
                if configured != partition.len() {
                    bail!(
                        "match.keys binds {configured} column(s) but the table has {} partition column(s)",
                        partition.len()
                    );
                }
                // Same rule as `bound`: a point the index cannot match misses
                // on every record, and an `ip` range needs `cast: ip` spelled
                // out because OTAP has no IP value.
                let want = match range_type {
                    lt::RangeType::Ip => AttributeLookupCast::Ip,
                    lt::RangeType::Integer => AttributeLookupCast::Integer,
                };
                match point.cast {
                    Some(cast) if cast != want => bail!(
                        "match.point.cast is {:?} but the table's range is {range_type:?}",
                        cast
                    ),
                    None if *range_type == lt::RangeType::Ip => {
                        bail!("an ip range needs an explicit `cast: ip` on match.point")
                    }
                    _ => {}
                }
                (
                    partition.iter().map(|c| bound(*c)).collect::<Result<_>>()?,
                    Some((point, *range_type)),
                )
            }
        };

        let outputs = rule
            .output
            .iter()
            .map(|(destination, column)| {
                let idx = read
                    .column(column)
                    .ok_or_else(|| anyhow!("output column {column:?} is not in the table"))?;
                let ty = schema
                    .value_type(idx)
                    .ok_or_else(|| anyhow!("output column {column:?} has no type"))?;
                Ok((SmolStr::new(destination), idx, ty))
            })
            .collect::<Result<_>>()?;

        Ok(Plan {
            keys,
            point,
            outputs,
        })
    }
}

fn run_rule(
    pl: &Payload,
    rule: &AttributeLookupRule,
    shared: Option<&BooleanArray>,
    node_id: Uuid,
    cache: &cache::Cache,
    writes: &mut Writes,
) -> Result<()> {
    let table = cache
        .lookup_table
        .get(&rule.lookup_table)
        .ok_or_else(|| anyhow!("lookup table {} is not configured", rule.lookup_table))?;
    // One guard per rule per batch; it flushes its tally on drop.
    let read = table.read_as(node_id);
    if read.generation() == 0 {
        // Not loaded yet. The reloader keeps retrying, so this is transient.
        trace!(lookup_table_id = %rule.lookup_table, "lookup table has not loaded yet");
        return Ok(());
    }
    // validate lookup rule to the cache table's schema
    let plan = Plan::resolve(&read, rule)?;

    let parent_ids = attr_input::rule_parent_ids(pl, &rule.table)?;
    let eligible = attr_input::eligible_parents(pl, shared, rule.when.as_ref(), &parent_ids)?;
    let is_eligible = |parent| eligible.as_ref().is_none_or(|set| set.contains(&parent));

    // An explicit null is not a probe key, so it reads as absent here —
    // unlike `attribute_collect`, which stores it as a null cell.
    let source = |input: &AttributeLookupInput| {
        attr_input::read_source(
            pl,
            &rule.table,
            input.table.as_ref(),
            &input.key,
            &parent_ids,
            false,
        )
    };
    let key_sources = plan
        .keys
        .iter()
        .map(|input| source(input))
        .collect::<Result<Vec<_>>>()?;
    let point_source = plan.point.map(|(input, _)| source(input)).transpose()?;

    let candidates =
        attr_input::candidates(key_sources.iter().chain(point_source.as_ref()), is_eligible);
    let mut converted = Vec::with_capacity(plan.keys.len());

    for parent in candidates {
        converted.clear();
        let mut usable = true;
        for (input, source) in plan.keys.iter().zip(&key_sources) {
            match source.get(parent).and_then(|v| key_part(v, input.cast)) {
                Some(part) => converted.push(part),
                None => {
                    usable = false;
                    break;
                }
            }
        }
        let point = match (&plan.point, &point_source) {
            (Some((input, range_type)), Some(source)) if usable => {
                match source
                    .get(parent)
                    .and_then(|v| range_point(v, input.cast, *range_type))
                {
                    Some(point) => Some(point),
                    None => {
                        usable = false;
                        None
                    }
                }
            }
            _ => None,
        };
        if !usable {
            read.note_key_error();
            continue;
        }

        let parts: SmallVec<[KeyRef<'_>; 4]> = converted
            .iter()
            .map(|cell| {
                KeyRef::try_from_cell(cell.as_ref()).expect("key conversion excludes doubles")
            })
            .collect();
        let probe = match point {
            Some(point) => Lookup::Range {
                partition: &parts,
                point,
            },
            None => Lookup::Hash(&parts),
        };
        // A miss is tallied by `lookup` itself and leaves every destination alone.
        let Some(row) = read.lookup(probe) else {
            continue;
        };
        for (destination, column, ty) in &plan.outputs {
            let cell = row.cell(*column);
            if cell.is_null() {
                continue;
            }
            writes
                .table(&rule.table)
                .entry(destination.clone())
                .or_insert_with(|| Values::new(*ty))
                .insert(parent, cell);
        }
    }
    Ok(())
}

/// Convert an attribute value into one probe key part.
///
/// Without a cast the value must already be the key column's type: the index
/// compares variants, so `"42"` never matches an integer key. Strings go
/// through [`CellRef::parse`], the parser the table was built with, so probe
/// and build agree — including IPv4-mapped IPv6.
fn key_part<'a>(
    value: &'a Value<'_>,
    cast: Option<AttributeLookupCast>,
) -> Option<attr_input::Converted<'a>> {
    // Null is never a key. Nor is a bare double — but `cast: string` renders
    // one, and `attribute_collect` stores exactly that rendering through the
    // same `convert`, so refusing it would leave those rows unprobeable.
    let rendered_as_text = cast == Some(AttributeLookupCast::String);
    if matches!(value, Value::Null) || (matches!(value, Value::Double(_)) && !rendered_as_text) {
        return None;
    }
    attr_input::convert(value, cast.map(cast_type))
}

/// Shared with [`Plan::resolve`], so the type a binding is checked against at
/// plan time is the one it actually yields.
fn cast_type(cast: AttributeLookupCast) -> lt::ValueType {
    match cast {
        AttributeLookupCast::Bool => lt::ValueType::Bool,
        AttributeLookupCast::Integer => lt::ValueType::Integer,
        AttributeLookupCast::String => lt::ValueType::String,
        AttributeLookupCast::Ip => lt::ValueType::Ip,
    }
}

fn range_point(
    value: &Value<'_>,
    cast: Option<AttributeLookupCast>,
    range_type: lt::RangeType,
) -> Option<RangePoint> {
    RangePoint::from_cell(key_part(value, cast)?.as_ref(), range_type)
}

// ---- writing back ----

fn apply(pl: &mut Payload, writes: Writes, overwrite: bool) -> Result<()> {
    let mut first_error = None;
    for (table, keys) in [
        (AttributeTable::Record, writes.record),
        (AttributeTable::Scope, writes.scope),
        (AttributeTable::Resource, writes.resource),
    ] {
        if keys.is_empty() {
            continue;
        }
        if let Err(error) = apply_table(pl, &table, &keys, overwrite) {
            // A record-ID failure must not block scope/resource writes.
            first_error.get_or_insert(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Merge one table's writes, committing a new root and its attributes together.
fn apply_table(
    pl: &mut Payload,
    table: &AttributeTable,
    keys: &TableWrites,
    overwrite: bool,
) -> Result<()> {
    let payload_type = attrs_payload_type(&pl.data, table);
    let existing = pl
        .data
        .get(payload_type)
        .unwrap_or(&EMPTY_U16_ATTRS_RECORD_BATCH);
    let (root, assigned) = assign_record_ids(pl, keys)?;
    let mut writes = Vec::with_capacity(keys.values().map(Values::len).sum());
    for (key, values) in keys {
        let mut parents = Vec::with_capacity(values.len());
        values.for_each_parent(|logical| {
            let encoded = match logical {
                Parent::Existing(id) => id,
                Parent::MissingRecord(row) => assigned[&row],
            };
            parents.push((encoded, logical));
        });
        parents.sort_unstable_by_key(|(encoded, _)| *encoded);
        writes.extend(
            parents
                .into_iter()
                .map(|(parent, logical)| super::attr_write::Write {
                    key: key.as_str(),
                    parent,
                    cell: values.cell(logical),
                }),
        );
    }
    let Some(merged) = super::attr_write::merge(existing, &writes, overwrite)? else {
        return Ok(());
    };
    // On an Arc-backed clone first: either both related batches are valid, or
    // the original payload comes back without dangling references.
    let mut data = pl.data.clone();
    data.set(payload_type, merged)
        .map_err(|e| anyhow!("set {}: {e}", table.as_str()))?;
    if let Some(root) = root {
        let root_type = match &data {
            OtapArrowRecords::Logs(_) => ArrowPayloadType::Logs,
            OtapArrowRecords::Metrics(_) => ArrowPayloadType::UnivariateMetrics,
            OtapArrowRecords::Traces(_) => ArrowPayloadType::Spans,
        };
        data.set(root_type, root)
            .map_err(|e| anyhow!("set record ids: {e}"))?;
    }
    pl.data = data;
    Ok(())
}

/// Allocate IDs only for ID-less records with pending writes. Existing IDs and
/// references are reserved; no populated ID is renumbered.
fn assign_record_ids(
    pl: &Payload,
    keys: &TableWrites,
) -> Result<(Option<RecordBatch>, FxHashMap<usize, u16>)> {
    let mut missing = std::collections::BTreeSet::new();
    for values in keys.values() {
        values.for_each_parent(|parent| {
            if let Parent::MissingRecord(row) = parent {
                missing.insert(row);
            }
        });
    }
    let mut assigned = FxHashMap::default();
    if missing.is_empty() {
        return Ok((None, assigned));
    }
    let record = attr::get_record(pl).ok_or_else(|| anyhow!("missing root record batch"))?;
    let mut ids = attr::location_parent_ids(pl, attr::AttrLocation::Record)?;
    // The complete UInt16 domain fits in an 8 KiB stack bitmap.
    let mut used = [0u64; 1024];
    let mut reserve = |id: u16| {
        used[id as usize / 64] |= 1u64 << (id % 64);
    };
    for id in ids.iter().flatten() {
        reserve(*id);
    }
    let children: &[ArrowPayloadType] = match &pl.data {
        OtapArrowRecords::Logs(_) => &[ArrowPayloadType::LogAttrs],
        OtapArrowRecords::Traces(_) => &[
            ArrowPayloadType::SpanAttrs,
            ArrowPayloadType::SpanEvents,
            ArrowPayloadType::SpanLinks,
        ],
        OtapArrowRecords::Metrics(_) => &[
            ArrowPayloadType::MetricAttrs,
            ArrowPayloadType::NumberDataPoints,
            ArrowPayloadType::SummaryDataPoints,
            ArrowPayloadType::HistogramDataPoints,
            ArrowPayloadType::ExpHistogramDataPoints,
        ],
    };
    for ty in children {
        if let Some(column) = pl
            .data
            .get(*ty)
            .and_then(|b| b.column_by_name(consts::PARENT_ID))
        {
            let parents = column
                .as_primitive_opt::<UInt16Type>()
                .ok_or_else(|| anyhow!("expected UInt16 parent ids in {ty:?}"))?;
            for id in parents.iter().flatten() {
                reserve(id);
            }
        }
    }
    let mut free = (0..=u16::MAX).filter(|id| used[*id as usize / 64] & (1u64 << (*id % 64)) == 0);
    for row in missing {
        let slot = ids
            .get_mut(row)
            .ok_or_else(|| anyhow!("record row {row} is out of bounds"))?;
        let id = free.next().ok_or_else(|| {
            anyhow!("record attribute IDs exhausted; leaving record attributes unchanged")
        })?;
        *slot = Some(id);
        assigned.insert(row, id);
    }
    let schema = record.schema();
    let mut fields = schema.fields().to_vec();
    let mut columns = record.columns().to_vec();
    let ids: ArrayRef = Arc::new(UInt16Array::from(ids));
    if let Ok(index) = schema.index_of(consts::ID) {
        columns[index] = ids;
    } else {
        fields.push(Arc::new(
            Field::new(consts::ID, DataType::UInt16, true).with_metadata(
                [(
                    consts::metadata::COLUMN_ENCODING.to_string(),
                    "plain".to_string(),
                )]
                .into(),
            ),
        ));
        columns.push(ids);
    }
    let root = RecordBatch::try_new(
        Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone())),
        columns,
    )?;
    Ok((Some(root), assigned))
}

// ---- small mappings ----

fn attrs_payload_type(data: &OtapArrowRecords, table: &AttributeTable) -> ArrowPayloadType {
    match table {
        AttributeTable::Resource => ArrowPayloadType::ResourceAttrs,
        AttributeTable::Scope => ArrowPayloadType::ScopeAttrs,
        // Log/span/metric attributes; data-point attributes are out of scope.
        AttributeTable::Record => match data {
            OtapArrowRecords::Logs(_) => ArrowPayloadType::LogAttrs,
            OtapArrowRecords::Metrics(_) => ArrowPayloadType::MetricAttrs,
            OtapArrowRecords::Traces(_) => ArrowPayloadType::SpanAttrs,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::lookup_table::mutable::Upsert;
    use crate::config::processor::attribute_lookup::AttributeLookup;
    use crate::payload::attr::{AttrValueCols, StrCol};
    use arrow::datatypes::UInt16Type;
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value as AnyVal;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, KeyValue};
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource;
    use serde_json::{Value as Json, json};
    use std::time::Duration;

    // ---- payload fixtures ----

    fn kv(key: &str, value: AnyVal) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue { value: Some(value) }),
        }
    }

    fn s(v: &str) -> AnyVal {
        AnyVal::StringValue(v.to_string())
    }

    fn int(v: i64) -> AnyVal {
        AnyVal::IntValue(v)
    }

    #[test]
    fn an_unowned_resource_does_not_borrow_resource_zero_for_enrichment() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("jkt")]],
        );
        let cfg = config(json!({"lookups":[{
            "lookup_table":id,"table":"record",
            "match":{"keys":{"host":{"table":"resource","key":"host"}}},
            "output":{"site":"site"}
        }]}));
        for null_struct in [false, true] {
            let pl = super::super::test_payloads::logs_with_unowned_resource(null_struct);
            let (out, errored) = run(pl, &cfg, &cache);
            assert!(!errored);
            let sites: Vec<_> = record_attrs(&out)
                .into_iter()
                .filter(|(_, key, _)| key == "site")
                .collect();
            assert_eq!(sites, vec![(0, "site".into(), "jkt".into())]);
        }
    }

    /// A logs payload with one resource and one scope. Every record carries at
    /// least one attribute, so the encoder assigns log ids 0, 1, …
    fn logs(resource: Vec<KeyValue>, records: Vec<Vec<KeyValue>>) -> Payload {
        let request = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: (!resource.is_empty()).then(|| Resource {
                    attributes: resource,
                    ..Default::default()
                }),
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: records
                        .into_iter()
                        .map(|attributes| LogRecord {
                            time_unix_nano: 1,
                            attributes,
                            ..Default::default()
                        })
                        .collect(),
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        Payload::new(encode_logs_otap_batch(&request).unwrap())
    }

    /// Every attribute row of one table as `(parent_id, key, rendered value)`,
    /// sorted so assertions do not depend on physical row order.
    fn dump(pl: &Payload, payload_type: ArrowPayloadType) -> Vec<(u16, String, String)> {
        let Some(batch) = pl.data.get(payload_type) else {
            return Vec::new();
        };
        let parents = batch
            .column_by_name(consts::PARENT_ID)
            .unwrap()
            .as_primitive::<UInt16Type>();
        let keys = StrCol::try_new(
            batch
                .column_by_name(consts::ATTRIBUTE_KEY)
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let values = AttrValueCols::try_new(batch).unwrap();
        let mut out: Vec<_> = (0..batch.num_rows())
            .map(|i| {
                let rendered = match values.value_at(i) {
                    Some(Value::String(v)) => v.as_str().to_string(),
                    Some(Value::Int64(v)) => format!("i{v}"),
                    Some(Value::Double(v)) => format!("d{v}"),
                    Some(Value::Bool(v)) => format!("b{v}"),
                    _ => "-".to_string(),
                };
                (parents.value(i), keys.value(i).to_string(), rendered)
            })
            .collect();
        out.sort();
        out
    }

    fn record_attrs(pl: &Payload) -> Vec<(u16, String, String)> {
        dump(pl, ArrowPayloadType::LogAttrs)
    }

    fn resource_attrs(pl: &Payload) -> Vec<(u16, String, String)> {
        dump(pl, ArrowPayloadType::ResourceAttrs)
    }

    // ---- lookup table fixtures ----

    fn processor_source(columns: &[(&str, lt::ValueType)]) -> lt::Source {
        lt::Source::Processor(lt::Processor {
            columns: columns
                .iter()
                .map(|(name, value_type)| lt::ColumnSpec {
                    name: name.to_string(),
                    value_type: *value_type,
                })
                .collect(),
            ttl: Duration::from_secs(60),
            max_rows: 1000,
        })
    }

    /// A processor-fed hash table, populated up front. `columns` is the full
    /// column list; the key columns are part of it.
    fn hash_table(
        cache: &cache::Cache,
        key_columns: &[&str],
        columns: &[(&str, lt::ValueType)],
        rows: &[&[CellRef<'_>]],
    ) -> Uuid {
        let config = lt::LookupTable {
            id: Uuid::now_v7(),
            name: "hash".into(),
            source: processor_source(columns),
            key: lt::Key::Hash(lt::Hash {
                key_columns: key_columns.iter().map(|c| c.to_string()).collect(),
            }),
        };
        let table = cache.lookup_table.insert(&config).unwrap();
        let mutable = table.as_mutable().unwrap();
        for row in rows {
            assert_eq!(mutable.table().upsert_hash(row).unwrap(), Upsert::Inserted);
        }
        config.id
    }

    /// A processor-fed range table. `columns` includes the two bound columns,
    /// which the index consumes, so `rows` supplies only the materialized cells.
    fn range_table(
        cache: &cache::Cache,
        range_type: lt::RangeType,
        partition_columns: &[&str],
        columns: &[(&str, lt::ValueType)],
        rows: &[(&[CellRef<'_>], RangePoint, RangePoint)],
    ) -> Uuid {
        let config = lt::LookupTable {
            id: Uuid::now_v7(),
            name: "range".into(),
            source: processor_source(columns),
            key: lt::Key::Range(lt::Range {
                range_type,
                partition_columns: partition_columns.iter().map(|c| c.to_string()).collect(),
                start_column: "lo".into(),
                start_inclusive: true,
                end_column: "hi".into(),
                end_inclusive: true,
            }),
        };
        let table = cache.lookup_table.insert(&config).unwrap();
        let mutable = table.as_mutable().unwrap();
        for (cells, start, end) in rows {
            let outcome = mutable
                .table()
                .upsert_range(cells, *start, true, *end, true)
                .unwrap();
            assert_eq!(outcome, Upsert::Inserted);
        }
        config.id
    }

    // ---- driving the processor ----

    fn config(value: Json) -> AttributeLookup {
        let parsed: AttributeLookup = serde_json::from_value(value).unwrap();
        parsed.validate().expect("test config must be valid");
        parsed
    }

    fn run(pl: Payload, cfg: &AttributeLookup, cache: &cache::Cache) -> (Payload, bool) {
        attribute_lookup(pl, cfg, Uuid::now_v7(), cache)
    }

    /// Runs with a known node id so the per-processor counters can be read back.
    fn run_counted(
        pl: Payload,
        cfg: &AttributeLookup,
        cache: &cache::Cache,
        table: Uuid,
    ) -> crate::common::internal_metrics::LookupTableProcessor {
        let node = Uuid::now_v7();
        let _ = attribute_lookup(pl, cfg, node, cache);
        cache
            .lookup_table
            .get(&table)
            .unwrap()
            .meta()
            .metrics
            .processor_snapshots()
            .into_iter()
            .find(|p| p.id == node)
            .unwrap_or_default()
    }

    fn predicate(table: &str, key: &str, value: &str) -> Json {
        json!({"match": {
            "table": table, "key": key, "key_is_regex": false,
            "op": "=", "value": {"string": value}
        }})
    }

    // ---- tests ----

    #[test]
    fn resource_broadcast_enriches_attribute_free_records_in_all_signals() {
        use otel_arrow_dfe_pdata::{OtlpProtoBytes, TryFromWithOptions};
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("jkt")]],
        );
        let cfg = config(json!({"lookups":[{
            "lookup_table":id,"table":"record","match":{"keys":{"host":{"table":"resource","key":"host"}}},"output":{"site":"site"}
        }]}));
        for pl in super::super::test_payloads::signals() {
            let original = pl.data.clone();
            let old_ids = attr::location_parent_ids(&pl, attr::AttrLocation::Record).unwrap();
            let (out, errored) = run(pl, &cfg, &cache);
            assert!(!errored);
            let ids = attr::location_parent_ids(&out, attr::AttrLocation::Record).unwrap();
            assert!(ids.iter().all(Option::is_some));
            for (old, new) in old_ids.iter().zip(&ids) {
                if old.is_some() {
                    assert_eq!(old, new);
                }
            }
            let attrs = dump(&out, attrs_payload_type(&out.data, &AttributeTable::Record));
            assert_eq!(attrs.len(), 2);
            for id in ids.iter().flatten() {
                assert!(attrs.contains(&(*id, "site".into(), "jkt".into())));
            }
            for child in [
                ArrowPayloadType::NumberDataPoints,
                ArrowPayloadType::SpanEvents,
                ArrowPayloadType::SpanLinks,
            ] {
                assert_eq!(original.get(child), out.data.get(child));
            }
            let bytes = OtlpProtoBytes::try_from_with_default(out.data).unwrap();
            // Decode the generated protobuf too, not just validate Arrow schemas.
            use prost::Message;
            match bytes {
                OtlpProtoBytes::ExportLogsRequest(bytes) => {
                    let decoded = LogsData::decode(bytes).unwrap();
                    assert!(
                        decoded.resource_logs[0].scope_logs[0]
                            .log_records
                            .iter()
                            .all(|r| r.attributes.iter().any(|a| a.key == "site"))
                    );
                }
                OtlpProtoBytes::ExportMetricsRequest(bytes) => {
                    use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::MetricsData;
                    let decoded = MetricsData::decode(bytes).unwrap();
                    assert_eq!(
                        decoded.resource_metrics[0].scope_metrics[0].metrics.len(),
                        2
                    );
                }
                OtlpProtoBytes::ExportTracesRequest(bytes) => {
                    use otel_arrow_dfe_pdata::proto::opentelemetry::trace::v1::TracesData;
                    let decoded = TracesData::decode(bytes).unwrap();
                    assert!(
                        decoded.resource_spans[0].scope_spans[0]
                            .spans
                            .iter()
                            .all(|s| s.attributes.iter().any(|a| a.key == "site")
                                && s.events.len() == 1
                                && s.links.len() == 1)
                    );
                }
            }
        }
    }

    #[test]
    fn mixed_and_all_null_ids_are_allocated_without_changing_existing_ids() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("jkt")]],
        );
        let cfg = config(json!({"lookups":[{
            "lookup_table":id,"table":"record","match":{"keys":{"host":{"table":"resource","key":"host"}}},"output":{"site":"site"}
        }]}));
        for all_null in [false, true] {
            let mut pl = logs(
                vec![kv("host", s("a"))],
                vec![vec![], vec![kv("existing", s("keep"))], vec![]],
            );
            if all_null {
                pl.data.remove(ArrowPayloadType::LogAttrs);
                let root = attr::get_record(&pl).unwrap();
                let mut columns = root.columns().to_vec();
                columns[root.schema().index_of(consts::ID).unwrap()] =
                    Arc::new(UInt16Array::from(vec![None; 3]));
                pl.data
                    .set(
                        ArrowPayloadType::Logs,
                        RecordBatch::try_new(root.schema(), columns).unwrap(),
                    )
                    .unwrap();
            }
            let before = attr::location_parent_ids(&pl, attr::AttrLocation::Record).unwrap();
            let (out, error) = run(pl, &cfg, &cache);
            assert!(!error);
            let after = attr::location_parent_ids(&out, attr::AttrLocation::Record).unwrap();
            assert_eq!(
                after
                    .iter()
                    .flatten()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                3
            );
            for (before, after) in before.iter().zip(&after) {
                if before.is_some() {
                    assert_eq!(before, after);
                }
            }
            assert_eq!(
                record_attrs(&out)
                    .iter()
                    .filter(|(_, k, _)| k == "site")
                    .count(),
                3
            );
            if !all_null {
                assert!(
                    record_attrs(&out)
                        .iter()
                        .any(|(_, k, v)| k == "existing" && v == "keep")
                );
            }
        }
    }

    #[test]
    fn missing_ids_are_not_created_for_misses_or_guard_rejections() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("jkt")]],
        );
        for (host, guard) in [
            ("miss", None),
            ("a", Some(predicate("resource", "host", "reject"))),
        ] {
            let pl = logs(vec![kv("host", s(host))], vec![vec![], vec![]]);
            let original = pl.data.clone();
            let cfg = config(json!({"when":guard,"lookups":[{
                "lookup_table":id,"table":"record","match":{"keys":{"host":{"table":"resource","key":"host"}}},"output":{"site":"site"}
            }]}));
            let (out, error) = run(pl, &cfg, &cache);
            assert!(!error);
            assert_eq!(out.data, original);
        }
    }

    #[test]
    fn resource_guards_select_idless_rows_and_rules_read_only_original_inputs() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("a")]],
        );
        let request = LogsData {
            resource_logs: ["a", "b"]
                .into_iter()
                .map(|host| ResourceLogs {
                    resource: Some(Resource {
                        attributes: vec![kv("host", s(host))],
                        ..Default::default()
                    }),
                    scope_logs: vec![ScopeLogs {
                        log_records: vec![LogRecord {
                            time_unix_nano: 1,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                })
                .collect(),
        };
        let pl = Payload::new(encode_logs_otap_batch(&request).unwrap());
        let cfg = config(
            json!({"when":predicate("resource", "host", "a"), "lookups":[
                {"lookup_table":id,"table":"record","when":predicate("resource", "host", "a"), "match":{"keys":{"host":{"table":"resource","key":"host"}}},"output":{"site":"site"}},
                {"lookup_table":id,"table":"record","match":{"keys":{"host":{"key":"site"}}},"output":{"must_not_appear":"site"}}
            ]}),
        );
        let (out, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert_eq!(
            attr::location_parent_ids(&out, attr::AttrLocation::Record).unwrap(),
            vec![Some(0), None]
        );
        assert_eq!(record_attrs(&out), vec![(0, "site".into(), "a".into())]);
    }

    #[test]
    fn exhausted_record_ids_leave_the_original_batches_untouched() {
        let mut pl = logs(vec![kv("host", s("a"))], vec![vec![]]);
        // Reserve every possible ID, including IDs referenced only by attributes.
        let attrs = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new(consts::PARENT_ID, DataType::UInt16, false),
                Field::new(consts::ATTRIBUTE_KEY, DataType::Utf8, false),
                Field::new(consts::ATTRIBUTE_TYPE, DataType::UInt8, false),
            ])),
            vec![
                Arc::new(UInt16Array::from_iter_values(0..=u16::MAX)),
                Arc::new(arrow::array::StringArray::from_iter_values(
                    std::iter::repeat_n("reserved", 65_536),
                )),
                Arc::new(arrow::array::UInt8Array::from(vec![0; 65_536])),
            ],
        )
        .unwrap();
        pl.data.set(ArrowPayloadType::LogAttrs, attrs).unwrap();
        let before = pl.data.clone();
        let mut values = Values::new(lt::ValueType::String);
        values.insert(Parent::MissingRecord(0), CellRef::String("new"));
        let keys = BTreeMap::from([(SmolStr::new("site"), values)]);
        assert!(
            apply_table(&mut pl, &AttributeTable::Record, &keys, true)
                .unwrap_err()
                .to_string()
                .contains("exhausted")
        );
        assert_eq!(pl.data, before);
    }

    /// The fixture table is deliberately empty, so this measures the *probe*
    /// path only — see `enriching_a_record_does_not_allocate_per_record` for
    /// what a hit costs.
    #[test]
    fn long_probe_strings_do_not_add_per_candidate_allocations() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[],
        );
        let cfg = config(
            json!({"lookups":[{"lookup_table":id,"table":"record","match":{"keys":{"host":{"key":"host"}}},"output":{"site":"site"}}]}),
        );
        let node = Uuid::now_v7();
        let mut counts = Vec::new();
        for key in [
            "short",
            "a very long hostname that exceeds the inline string capacity",
        ] {
            let pl = logs(
                vec![],
                (0..1000).map(|_| vec![kv("host", s(key))]).collect(),
            );
            let _ = attribute_lookup(pl.clone(), &cfg, node, &cache);
            let ((_, errored), allocations) =
                super::super::test_alloc::measure(|| attribute_lookup(pl, &cfg, node, &cache));
            assert!(!errored);
            counts.push(allocations.calls);
        }
        assert_eq!(counts[0], counts[1]);
    }

    /// `attribute_collect` renders a double through `cast: string` and stores
    /// the result, so the mirror lookup has to be able to probe for it. Refusing
    /// every double before even reading the cast left those rows unreachable.
    #[test]
    fn a_double_rendered_by_cast_string_probes_the_row_collect_would_store() {
        let cache = cache::Cache::new();
        // "1.5" is exactly what `attr_input::convert` renders for Double(1.5),
        // which is what the collect half writes
        let id = hash_table(
            &cache,
            &["bucket"],
            &[
                ("bucket", lt::ValueType::String),
                ("tier", lt::ValueType::String),
            ],
            &[&[CellRef::String("1.5"), CellRef::String("warm")]],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"bucket": {"key": "ratio", "cast": "string"}}},
            "output": {"tier": "tier"},
        }]}));
        let pl = logs(vec![], vec![vec![kv("ratio", AnyVal::DoubleValue(1.5))]]);

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert!(
            record_attrs(&pl).contains(&(0, "tier".into(), "warm".into())),
            "{:?}",
            record_attrs(&pl)
        );
    }

    /// A hit used to cost one heap allocation per record, `Values::insert`
    /// taking an owned `String` of every matched cell; `SmolStr` keeps those
    /// inline. Asserted as a growth rate, so the fixed per-batch overhead may
    /// change but the slope must stay near zero — one-per-record reads ~900.
    #[test]
    fn enriching_a_record_does_not_allocate_per_record() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("h"), CellRef::String("dc1")]],
        );
        let cfg = config(
            json!({"lookups":[{"lookup_table":id,"table":"record","match":{"keys":{"host":{"key":"host"}}},"output":{"site":"site"}}]}),
        );
        let node = Uuid::now_v7();
        let measure = |records: usize| {
            let pl = logs(
                vec![],
                (0..records).map(|_| vec![kv("host", s("h"))]).collect(),
            );
            // once to warm every lazily grown map, then measure a clean run
            let _ = attribute_lookup(pl.clone(), &cfg, node, &cache);
            let ((pl, errored), allocations) =
                super::super::test_alloc::measure(|| attribute_lookup(pl, &cfg, node, &cache));
            assert!(!errored);
            assert_eq!(
                record_attrs(&pl)
                    .iter()
                    .filter(|(_, k, _)| k == "site")
                    .count(),
                records,
                "every record must actually have been enriched"
            );
            allocations.calls
        };

        let (small, large) = (measure(100), measure(1000));
        assert!(
            large.saturating_sub(small) < 200,
            "900 more records must not cost ~900 more allocations: {small} -> {large}"
        );
    }

    #[test]
    fn a_scalar_hash_lookup_adds_one_attribute_per_record() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[
                &[CellRef::String("a"), CellRef::String("jkt")],
                &[CellRef::String("b"), CellRef::String("sby")],
            ],
        );
        let pl = logs(
            vec![],
            vec![vec![kv("host", s("a"))], vec![kv("host", s("b"))]],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id,
            "table": "record",
            "match": {"keys": {"host": {"key": "host"}}},
            "output": {"site": "site"},
        }]}));

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert_eq!(
            record_attrs(&pl),
            vec![
                (0, "host".into(), "a".into()),
                (0, "site".into(), "jkt".into()),
                (1, "host".into(), "b".into()),
                (1, "site".into(), "sby".into()),
            ]
        );
    }

    #[test]
    fn composite_keys_bind_by_column_name_not_by_config_order() {
        let cache = cache::Cache::new();
        // `tenant` is the table's *second* key column but sorts first in the
        // config's BTreeMap, so a positional binding would swap the tuple.
        let id = hash_table(
            &cache,
            &["site", "tenant"],
            &[
                ("site", lt::ValueType::String),
                ("tenant", lt::ValueType::Integer),
                ("owner", lt::ValueType::String),
            ],
            &[&[
                CellRef::String("jkt"),
                CellRef::Integer(7),
                CellRef::String("neteng"),
            ]],
        );
        let pl = logs(
            vec![],
            vec![vec![kv("site", s("jkt")), kv("tenant", int(7))]],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id,
            "table": "record",
            "match": {"keys": {
                "tenant": {"key": "tenant"},
                "site": {"key": "site"},
            }},
            "output": {"owner": "owner"},
        }]}));

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert!(record_attrs(&pl).contains(&(0, "owner".into(), "neteng".into())));
    }

    #[test]
    fn a_record_rule_reads_a_resource_attribute_through_the_resource_id() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("team", lt::ValueType::String),
            ],
            &[&[CellRef::String("edge-1"), CellRef::String("core")]],
        );
        let pl = logs(
            vec![kv("host.name", s("edge-1"))],
            vec![vec![kv("msg", s("one"))], vec![kv("msg", s("two"))]],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id,
            "table": "record",
            "match": {"keys": {"host": {"table": "resource", "key": "host.name"}}},
            "output": {"team": "team"},
        }]}));

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        // Both records share the one resource, so both get enriched.
        assert_eq!(
            record_attrs(&pl),
            vec![
                (0, "msg".into(), "one".into()),
                (0, "team".into(), "core".into()),
                (1, "msg".into(), "two".into()),
                (1, "team".into(), "core".into()),
            ]
        );
    }

    #[test]
    fn a_resource_destination_writes_once_for_the_shared_parent() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("team", lt::ValueType::String),
            ],
            &[&[CellRef::String("edge-1"), CellRef::String("core")]],
        );
        let pl = logs(
            vec![kv("host.name", s("edge-1"))],
            vec![vec![kv("msg", s("one"))], vec![kv("msg", s("two"))]],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id,
            "table": "resource",
            "match": {"keys": {"host": {"key": "host.name"}}},
            "output": {"owner.team": "team"},
        }]}));

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert_eq!(
            resource_attrs(&pl),
            vec![
                (0, "host.name".into(), "edge-1".into()),
                (0, "owner.team".into(), "core".into()),
            ],
            "two records share one resource, so exactly one row is added"
        );
    }

    #[test]
    fn unpartitioned_and_partitioned_integer_ranges_both_probe() {
        for partitioned in [false, true] {
            let cache = cache::Cache::new();
            let columns: Vec<(&str, lt::ValueType)> = if partitioned {
                vec![
                    ("lo", lt::ValueType::Integer),
                    ("hi", lt::ValueType::Integer),
                    ("site", lt::ValueType::String),
                    ("zone", lt::ValueType::String),
                ]
            } else {
                vec![
                    ("lo", lt::ValueType::Integer),
                    ("hi", lt::ValueType::Integer),
                    ("zone", lt::ValueType::String),
                ]
            };
            let partition: &[&str] = if partitioned { &["site"] } else { &[] };
            let cells: Vec<CellRef<'_>> = if partitioned {
                vec![CellRef::String("jkt"), CellRef::String("dmz")]
            } else {
                vec![CellRef::String("dmz")]
            };
            let id = range_table(
                &cache,
                lt::RangeType::Integer,
                partition,
                &columns,
                &[(&cells, RangePoint::Integer(100), RangePoint::Integer(199))],
            );

            let mut attrs = vec![kv("vlan", int(150))];
            let mut keys = json!({});
            if partitioned {
                attrs.push(kv("site", s("jkt")));
                keys = json!({"site": {"key": "site"}});
            }
            let pl = logs(vec![], vec![attrs]);
            let cfg = config(json!({"lookups": [{
                "lookup_table": id,
                "table": "record",
                "match": {"keys": keys, "point": {"key": "vlan"}},
                "output": {"zone": "zone"},
            }]}));

            let (pl, errored) = run(pl, &cfg, &cache);
            assert!(!errored, "partitioned={partitioned}");
            assert!(
                record_attrs(&pl).contains(&(0, "zone".into(), "dmz".into())),
                "partitioned={partitioned}: {:?}",
                record_attrs(&pl)
            );
        }
    }

    #[test]
    fn an_ip_range_probe_canonicalises_the_v4_mapped_form() {
        let cache = cache::Cache::new();
        let id = range_table(
            &cache,
            lt::RangeType::Ip,
            &[],
            &[
                ("lo", lt::ValueType::Ip),
                ("hi", lt::ValueType::Ip),
                ("country", lt::ValueType::String),
            ],
            &[(
                &[CellRef::String("ID")],
                RangePoint::Ip("10.0.0.0".parse().unwrap()),
                RangePoint::Ip("10.0.0.255".parse().unwrap()),
            )],
        );
        // The mapped form must fold onto v4, or the probe would look in the v6
        // set and miss.
        let pl = logs(
            vec![],
            vec![
                vec![kv("flow.src", s("10.0.0.7"))],
                vec![kv("flow.src", s("::ffff:10.0.0.9"))],
                vec![kv("flow.src", s("11.0.0.1"))],
            ],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id,
            "table": "record",
            "match": {"point": {"key": "flow.src", "cast": "ip"}},
            "output": {"flow.country": "country"},
        }]}));

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        let attrs = record_attrs(&pl);
        assert!(
            attrs.contains(&(0, "flow.country".into(), "ID".into())),
            "{attrs:?}"
        );
        assert!(
            attrs.contains(&(1, "flow.country".into(), "ID".into())),
            "{attrs:?}"
        );
        assert!(
            !attrs.iter().any(|(p, k, _)| *p == 2 && k == "flow.country"),
            "an out-of-range address must not be enriched: {attrs:?}"
        );
    }

    /// An `ip` key column without `cast: ip` used to plan fine and miss every
    /// record: the attribute arrives as a string and hashes as
    /// `KeyRef::String` while the row is indexed as `KeyRef::Ip`, which looked
    /// exactly like a table holding no match.
    #[test]
    fn an_ip_key_column_without_a_cast_is_a_plan_error_not_a_silent_total_miss() {
        let cache = cache::Cache::new();
        let ip: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        let id = hash_table(
            &cache,
            &["ip"],
            &[("ip", lt::ValueType::Ip), ("site", lt::ValueType::String)],
            &[&[CellRef::Ip(ip), CellRef::String("jkt")]],
        );
        let payload = || logs(vec![], vec![vec![kv("ip", s("10.0.0.1"))]]);

        let uncast = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"ip": {"key": "ip"}}},
            "output": {"site": "site"},
        }]}));
        let (pl, errored) = run(payload(), &uncast, &cache);
        assert!(errored, "an uncast ip key must fail the rule");
        assert!(
            !record_attrs(&pl).iter().any(|(_, k, _)| k == "site"),
            "a plan error must not enrich"
        );

        // spelled out, the same probe hits
        let cast = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"ip": {"key": "ip", "cast": "ip"}}},
            "output": {"site": "site"},
        }]}));
        let (pl, errored) = run(payload(), &cast, &cache);
        assert!(!errored);
        assert!(
            record_attrs(&pl).contains(&(0, "site".into(), "jkt".into())),
            "{:?}",
            record_attrs(&pl)
        );
    }

    /// A cast that cannot produce the key column's type is a config mistake, so
    /// it is reported once at plan time rather than as a miss per record.
    #[test]
    fn a_key_cast_that_contradicts_the_column_type_is_a_plan_error() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["port"],
            &[
                ("port", lt::ValueType::Integer),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::Integer(8080), CellRef::String("jkt")]],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"port": {"key": "port", "cast": "string"}}},
            "output": {"site": "site"},
        }]}));
        let (_, errored) = run(
            logs(vec![], vec![vec![kv("port", int(8080))]]),
            &cfg,
            &cache,
        );
        assert!(errored);
    }

    /// The check lives in `bound`, which both key kinds go through, so a range
    /// table's partition columns are covered by the same rule as a hash key's.
    #[test]
    fn a_range_partition_column_is_checked_the_same_way_as_a_hash_key() {
        let cache = cache::Cache::new();
        let id = range_table(
            &cache,
            lt::RangeType::Integer,
            &["ip"],
            &[
                ("ip", lt::ValueType::Ip),
                ("lo", lt::ValueType::Integer),
                ("hi", lt::ValueType::Integer),
                ("country", lt::ValueType::String),
            ],
            &[(
                &[
                    CellRef::Ip("10.0.0.1".parse().unwrap()),
                    CellRef::String("ID"),
                ],
                RangePoint::Integer(1),
                RangePoint::Integer(10),
            )],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {
                "keys": {"ip": {"key": "ip"}},
                "point": {"key": "n", "cast": "integer"},
            },
            "output": {"country": "country"},
        }]}));
        let pl = logs(vec![], vec![vec![kv("ip", s("10.0.0.1")), kv("n", int(5))]]);
        let (_, errored) = run(pl, &cfg, &cache);
        assert!(errored, "an uncast ip partition column must fail the rule");
    }

    #[test]
    fn overwrite_replaces_an_existing_key_and_insert_only_leaves_it() {
        for overwrite in [false, true] {
            let cache = cache::Cache::new();
            let id = hash_table(
                &cache,
                &["host"],
                &[
                    ("host", lt::ValueType::String),
                    ("site", lt::ValueType::String),
                ],
                &[&[CellRef::String("a"), CellRef::String("jkt")]],
            );
            // Record 0 already has `site`; record 1 does not.
            let pl = logs(
                vec![],
                vec![
                    vec![kv("host", s("a")), kv("site", s("stale"))],
                    vec![kv("host", s("a"))],
                ],
            );
            let cfg = config(json!({
                "overwrite": overwrite,
                "lookups": [{
                    "lookup_table": id,
                    "table": "record",
                    "match": {"keys": {"host": {"key": "host"}}},
                    "output": {"site": "site"},
                }],
            }));

            let (pl, errored) = run(pl, &cfg, &cache);
            assert!(!errored);
            let attrs = record_attrs(&pl);
            let expected = if overwrite { "jkt" } else { "stale" };
            assert_eq!(
                attrs
                    .iter()
                    .filter(|(p, k, _)| *p == 0 && k == "site")
                    .count(),
                1,
                "the existing key must not be duplicated: {attrs:?}"
            );
            assert!(
                attrs.contains(&(0, "site".into(), expected.into())),
                "overwrite={overwrite}: {attrs:?}"
            );
            assert!(
                attrs.contains(&(1, "site".into(), "jkt".into())),
                "a parent without the key is always filled: {attrs:?}"
            );
        }
    }

    #[test]
    fn a_null_result_cell_leaves_the_destination_untouched() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::Null]],
        );
        let pl = logs(vec![], vec![vec![kv("host", s("a"))]]);
        let cfg = config(json!({"lookups": [{
            "lookup_table": id,
            "table": "record",
            "match": {"keys": {"host": {"key": "host"}}},
            "output": {"site": "site"},
        }]}));

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert_eq!(
            record_attrs(&pl),
            vec![(0, "host".into(), "a".into())],
            "a hit whose cell is null writes nothing"
        );
    }

    #[test]
    fn guards_narrow_which_records_are_enriched() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("jkt")]],
        );
        let pl = logs(
            vec![],
            vec![
                vec![kv("host", s("a")), kv("env", s("prod"))],
                vec![kv("host", s("a")), kv("env", s("dev"))],
            ],
        );
        let cfg = config(json!({
            "when": predicate("record", "env", "prod"),
            "lookups": [{
                "lookup_table": id,
                "table": "record",
                "match": {"keys": {"host": {"key": "host"}}},
                "output": {"site": "site"},
            }],
        }));

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        let attrs = record_attrs(&pl);
        assert!(
            attrs.contains(&(0, "site".into(), "jkt".into())),
            "{attrs:?}"
        );
        assert!(
            !attrs.iter().any(|(p, k, _)| *p == 1 && k == "site"),
            "an unselected record passes through unchanged: {attrs:?}"
        );
    }

    #[test]
    fn a_rule_guard_is_anded_with_the_shared_one() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("jkt")]],
        );
        let pl = logs(
            vec![],
            vec![
                vec![
                    kv("host", s("a")),
                    kv("env", s("prod")),
                    kv("tier", s("edge")),
                ],
                vec![
                    kv("host", s("a")),
                    kv("env", s("prod")),
                    kv("tier", s("core")),
                ],
            ],
        );
        let mut rule = json!({
            "lookup_table": id,
            "table": "record",
            "match": {"keys": {"host": {"key": "host"}}},
            "output": {"site": "site"},
        });
        rule["when"] = predicate("record", "tier", "edge");
        let cfg = config(json!({
            "when": predicate("record", "env", "prod"),
            "lookups": [rule],
        }));

        let (pl, _) = run(pl, &cfg, &cache);
        let attrs = record_attrs(&pl);
        assert!(
            attrs.contains(&(0, "site".into(), "jkt".into())),
            "{attrs:?}"
        );
        assert!(
            !attrs.iter().any(|(p, k, _)| *p == 1 && k == "site"),
            "the rule guard must further narrow the shared one: {attrs:?}"
        );
    }

    #[test]
    fn casts_reuse_the_parser_the_table_was_built_with() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["idx"],
            &[
                ("idx", lt::ValueType::Integer),
                ("name", lt::ValueType::String),
            ],
            &[&[CellRef::Integer(7), CellRef::String("seven")]],
        );
        // The attribute is a string; without `cast: integer` the variant-matched
        // index would never match an integer key column.
        let pl = logs(vec![], vec![vec![kv("idx", s("7"))]]);
        let uncast = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"idx": {"key": "idx"}}},
            "output": {"name": "name"},
        }]}));
        let (plain, _) = run(pl.clone(), &uncast, &cache);
        assert!(
            !record_attrs(&plain).iter().any(|(_, k, _)| k == "name"),
            "a string must not match an integer key without a cast"
        );

        let cast = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"idx": {"key": "idx", "cast": "integer"}}},
            "output": {"name": "name"},
        }]}));
        let (converted, _) = run(pl, &cast, &cache);
        assert!(record_attrs(&converted).contains(&(0, "name".into(), "seven".into())));
    }

    #[test]
    fn hits_misses_and_key_errors_are_counted_separately() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["idx"],
            &[
                ("idx", lt::ValueType::Integer),
                ("name", lt::ValueType::String),
            ],
            &[&[CellRef::Integer(1), CellRef::String("one")]],
        );
        let pl = logs(
            vec![],
            vec![
                vec![kv("idx", s("1"))],    // hit
                vec![kv("idx", s("2"))],    // probe built, no row  -> miss
                vec![kv("idx", s("nope"))], // uncastable           -> key error
                vec![kv("other", s("x"))],  // not participating    -> nothing
            ],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"idx": {"key": "idx", "cast": "integer"}}},
            "output": {"name": "name"},
        }]}));

        let m = run_counted(pl, &cfg, &cache, id);
        assert_eq!(m.lookups_hit, 1);
        assert_eq!(m.lookups_missed, 1);
        assert_eq!(m.lookups_errored, 1);
        assert_eq!(
            m.lookups, 3,
            "a record with none of the inputs is not probed"
        );
    }

    #[test]
    fn a_misconfigured_rule_is_skipped_without_dropping_records() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("jkt")]],
        );
        let pl = logs(vec![], vec![vec![kv("host", s("a"))]]);
        let cfg = config(json!({"lookups": [{
            "lookup_table": id,
            "table": "record",
            "match": {"keys": {"host": {"key": "host"}}},
            "output": {"site": "no_such_column"},
        }]}));

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(errored, "an unresolvable output column is an error");
        assert_eq!(
            record_attrs(&pl),
            vec![(0, "host".into(), "a".into())],
            "the records survive untouched"
        );
    }

    #[test]
    fn an_unconfigured_table_errors_and_an_unloaded_one_does_not() {
        let cache = cache::Cache::new();
        let pl = logs(vec![], vec![vec![kv("host", s("a"))]]);

        let missing = config(json!({"lookups": [{
            "lookup_table": Uuid::now_v7(),
            "table": "record",
            "match": {"keys": {"host": {"key": "host"}}},
            "output": {"site": "site"},
        }]}));
        let (pl, errored) = run(pl, &missing, &cache);
        assert!(
            errored,
            "a rule naming an unknown table is a misconfiguration"
        );

        // A file-backed table that has not loaded yet is transient: the reloader
        // keeps retrying, so it must not be reported as an error every batch.
        let unloaded = lt::LookupTable {
            id: Uuid::now_v7(),
            name: "geo".into(),
            source: lt::Source::File(lt::File {
                format: lt::Format::Csv(lt::Csv {
                    type_cast: Default::default(),
                    delimiter: ',',
                    quote: '"',
                    escape: None,
                    null_values: vec![String::new()],
                    headers: vec![],
                    has_headers: Some(true),
                }),
                paths: vec!["/nonexistent.csv".into()],
                compression: lt::Compression::None,
                location: lt::Location::FileSystem,
                reload_interval: Duration::from_secs(60),
            }),
            key: lt::Key::Hash(lt::Hash {
                key_columns: vec!["host".into()],
            }),
        };
        cache.lookup_table.insert(&unloaded).unwrap();
        let cfg = config(json!({"lookups": [{
            "lookup_table": unloaded.id,
            "table": "record",
            "match": {"keys": {"host": {"key": "host"}}},
            "output": {"site": "site"},
        }]}));
        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored, "a not-yet-loaded table is a no-op, not an error");
        assert_eq!(record_attrs(&pl), vec![(0, "host".into(), "a".into())]);
    }

    #[test]
    fn independent_rules_merge_into_one_rebuild_per_table() {
        let cache = cache::Cache::new();
        let sites = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("jkt")]],
        );
        let owners = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("team", lt::ValueType::String),
                ("oncall", lt::ValueType::Bool),
            ],
            &[&[
                CellRef::String("a"),
                CellRef::String("neteng"),
                CellRef::Bool(true),
            ]],
        );
        let pl = logs(
            vec![kv("host.name", s("a"))],
            vec![vec![kv("host", s("a"))]],
        );
        let cfg = config(json!({"lookups": [
            {
                "lookup_table": sites, "table": "record",
                "match": {"keys": {"host": {"key": "host"}}},
                "output": {"site": "site"},
            },
            {
                "lookup_table": owners, "table": "resource",
                "match": {"keys": {"host": {"key": "host.name"}}},
                "output": {"owner.team": "team", "owner.oncall": "oncall"},
            },
        ]}));

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert_eq!(
            record_attrs(&pl),
            vec![
                (0, "host".into(), "a".into()),
                (0, "site".into(), "jkt".into()),
            ]
        );
        assert_eq!(
            resource_attrs(&pl),
            vec![
                (0, "host.name".into(), "a".into()),
                (0, "owner.oncall".into(), "btrue".into()),
                (0, "owner.team".into(), "neteng".into()),
            ],
            "a bool output lands in the bool column, a string in the str column"
        );
    }

    #[test]
    fn a_missing_destination_batch_is_rebuilt_with_its_required_columns() {
        // Defensive: the config rules make this unreachable today (a rule needs
        // inputs from the table it writes to, bar the resource broadcast), but a
        // rebuild that dropped parent_id/type/key would be silently corrupt.
        let mut pl = logs(vec![kv("host.name", s("a"))], vec![vec![kv("msg", s("x"))]]);
        pl.data.remove(ArrowPayloadType::ResourceAttrs);
        assert!(pl.data.get(ArrowPayloadType::ResourceAttrs).is_none());

        let mut values = Values::new(lt::ValueType::String);
        values.insert(Parent::Existing(0), CellRef::String("jkt"));
        let mut writes: TableWrites = BTreeMap::new();
        writes.insert(SmolStr::new("site"), values);
        apply_table(&mut pl, &AttributeTable::Resource, &writes, false).unwrap();

        let batch = pl.data.get(ArrowPayloadType::ResourceAttrs).unwrap();
        for column in [
            consts::PARENT_ID,
            consts::ATTRIBUTE_TYPE,
            consts::ATTRIBUTE_KEY,
        ] {
            assert!(
                batch.column_by_name(column).is_some(),
                "{column} must survive the rebuild"
            );
        }
        assert_eq!(resource_attrs(&pl), vec![(0, "site".into(), "jkt".into())]);
    }

    #[test]
    fn an_empty_payload_is_returned_untouched() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("jkt")]],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"host": {"key": "host"}}},
            "output": {"site": "site"},
        }]}));
        let (pl, errored) = run(logs(vec![], vec![]), &cfg, &cache);
        assert!(!errored);
        assert!(pl.is_empty());
    }
}
