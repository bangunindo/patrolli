// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The `attribute_lookup` processor: batched attribute enrichment from the
//! configured lookup tables. Config contract on
//! [`patrolli::config::processor::attribute_lookup`]; this module is its execution.
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
use super::attr_write::{Values, Writes, apply, put};
use anyhow::{Result, anyhow, bail};
use arrow::array::BooleanArray;
use patrolli::cache;
use patrolli::cache::lookup_table::TableRead;
use patrolli::cache::lookup_table::cell::{CellRef, KeyOwned, KeyRef, Lookup};
use patrolli::cache::lookup_table::schema::{ColumnIdx, KeyPlan};
use patrolli::config::lookup_table as lt;
use patrolli::config::processor::attribute_lookup::{
    AttributeLookup, AttributeLookupCast, AttributeLookupInput, AttributeLookupRule,
};
use patrolli::payload::{self, Payload, Value, attr};
use regex::{CaptureLocations, Regex};
use smallvec::SmallVec;
use smol_str::{SmolStr, format_smolstr};
use std::net::IpAddr;
use tracing::{trace, warn};
use uuid::Uuid;

/// Enrich `pl` in place and report whether any rule was skipped by an error.
/// No failing return: the payload always comes back, enriched as far as it got.
pub(super) fn attribute_lookup(
    mut pl: payload::Payload,
    config: &AttributeLookup,
    node_id: Option<Uuid>,
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

// ---- one rule ----

/// A rule's config resolved against one generation of its lookup table.
struct Plan<'a> {
    /// In the table's own key order, which is what `Lookup` expects — the config
    /// binds them by name, so YAML map order is irrelevant.
    keys: Vec<Binding<'a>>,
    /// `Some` exactly when the table is prefix-keyed.
    point: Option<&'a AttributeLookupInput>,
    /// Destination keys are owned once here rather than per hit: `Writes` keys
    /// on `SmolStr`, and a long destination would otherwise allocate per record.
    outputs: Vec<(SmolStr, Output, lt::ValueType)>,
}

/// What supplies one key part.
#[derive(Debug)]
enum Binding<'a> {
    /// read per record from an attribute
    Attribute(&'a AttributeLookupInput),
    /// the same for every record, parsed once as the column's type
    Literal(KeyOwned),
}

/// Where an output's value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Output {
    Column(ColumnIdx),
    /// the prefix that matched, as CIDR text; named in `output` by the prefix
    /// column, which is consumed by the index and so is never a column
    MatchedPrefix,
}

impl<'a> Plan<'a> {
    /// `key` is the table's configured key: the prefix column's name lives only
    /// there, since the index consumes the column.
    fn resolve(read: &TableRead, key: &lt::Key, rule: &'a AttributeLookupRule) -> Result<Self> {
        let schema = read.schema();
        // The prefix column is consumed by the index and never materialized,
        // so it is not in `schema` and cannot be bound here.
        let bound = |idx: ColumnIdx| -> Result<Binding<'a>> {
            let name = schema
                .name(idx)
                .ok_or_else(|| anyhow!("key column {idx} is missing from the table schema"))?;
            let ty = schema
                .value_type(idx)
                .ok_or_else(|| anyhow!("key column {name:?} has no type"))?;
            if let Some(literal) = rule.match_by.literals.get(name) {
                // Parsed by the parser the table's rows went through, so the
                // constant and the stored key agree — v4-mapped IPv6 included.
                let text = literal.to_text();
                let key = CellRef::parse(&text, ty)
                    .and_then(KeyRef::try_from_cell)
                    .map_err(|e| {
                        anyhow!("match.literals[{name:?}] is not a valid {ty:?}: {e:#}")
                    })?;
                return Ok(Binding::Literal(key.to_owned_key()));
            }
            let input = rule
                .match_by
                .keys
                .get(name)
                .ok_or_else(|| anyhow!("match.keys has no binding for key column {name:?}"))?;
            // The index compares variants, so a wrong-typed binding misses on
            // every record. Fail the plan once instead of counting a miss per
            // record. In `bound`, not in one arm: a prefix table's partition
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
                // Extracted text stays text without a cast, and would miss
                // every record of a non-string column.
                (None, ty) if input.key.extract().is_some() && ty != lt::ValueType::String => {
                    bail!(
                        "match.keys[{name:?}] extracts text, so a {ty:?} key column needs a `cast`"
                    )
                }
                _ => {}
            }
            Ok(Binding::Attribute(input))
        };
        let configured = rule.match_by.keys.len() + rule.match_by.literals.len();
        let (keys, point) = match read.key_plan() {
            KeyPlan::Hash { columns } => {
                if rule.match_by.point.is_some() {
                    bail!("match.point is set but the lookup table is hash-keyed");
                }
                if configured != columns.len() {
                    bail!(
                        "match binds {configured} column(s) but the table's hash key has {}",
                        columns.len()
                    );
                }
                let keys: Vec<Binding> =
                    columns.iter().map(|c| bound(*c)).collect::<Result<_>>()?;
                // Every record would probe the same row, and with no attribute
                // to read no record would be a candidate anyway.
                if keys.iter().all(|k| matches!(k, Binding::Literal(_))) {
                    bail!("a hash key needs at least one column bound in match.keys");
                }
                (keys, None)
            }
            KeyPlan::Prefix { partition } => {
                let point = rule.match_by.point.as_ref().ok_or_else(|| {
                    anyhow!("the lookup table is prefix-keyed but match.point is not set")
                })?;
                if configured != partition.len() {
                    bail!(
                        "match binds {configured} column(s) but the table has {} partition column(s)",
                        partition.len()
                    );
                }
                // The point is always read as an address, so a cast, if
                // spelled out, can only say so.
                if let Some(cast) = point.cast
                    && cast != AttributeLookupCast::Ip
                {
                    bail!("match.point.cast is {cast:?} but a prefix table is probed by an ip");
                }
                (
                    partition.iter().map(|c| bound(*c)).collect::<Result<_>>()?,
                    Some(point),
                )
            }
        };

        let prefix_column = match key {
            lt::Key::Prefix(prefix) if point.is_some() => Some(prefix.prefix_column.as_str()),
            _ => None,
        };
        let outputs = rule
            .output
            .iter()
            .map(|(destination, column)| {
                if Some(column.as_str()) == prefix_column {
                    // emitted as CIDR text, like every IP value
                    return Ok((
                        SmolStr::new(destination),
                        Output::MatchedPrefix,
                        lt::ValueType::String,
                    ));
                }
                let idx = read
                    .column(column)
                    .ok_or_else(|| anyhow!("output column {column:?} is not in the table"))?;
                let ty = schema
                    .value_type(idx)
                    .ok_or_else(|| anyhow!("output column {column:?} has no type"))?;
                Ok((SmolStr::new(destination), Output::Column(idx), ty))
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
    node_id: Option<Uuid>,
    cache: &cache::Cache,
    writes: &mut Writes,
) -> Result<()> {
    let table = cache
        .lookup_table
        .get(&rule.lookup_table)
        .ok_or_else(|| anyhow!("lookup table {} is not configured", rule.lookup_table))?;
    // One guard per rule per batch; it flushes its tally on drop.
    let read = match node_id {
        Some(id) => table.read_as(id),
        None => table.read(),
    };
    if read.generation() == 0 {
        // Not loaded yet. The reloader keeps retrying, so this is transient.
        trace!(lookup_table_id = %rule.lookup_table, "lookup table has not loaded yet");
        return Ok(());
    }
    // validate lookup rule to the cache table's schema
    let plan = Plan::resolve(&read, &table.meta().config.key, rule)?;

    let parent_ids = attr_input::rule_parent_ids(pl, &rule.table)?;
    let eligible = attr_input::eligible_parents(pl, shared, rule.when.as_ref(), &parent_ids)?;
    let is_eligible = |parent| eligible.as_ref().is_none_or(|set| set.contains(&parent));

    // An explicit null is not a probe key, so it reads as absent here —
    // unlike `attribute_collect`, which stores it as a null cell.
    let source = |input: &AttributeLookupInput| {
        let read = |key: &String| {
            attr_input::read_source(
                pl,
                &rule.table,
                input.table.as_ref(),
                key,
                &parent_ids,
                false,
            )
        };
        match input.key.as_slice() {
            [key] => read(key),
            keys => Ok(attr_input::Source::FirstOf(
                keys.iter().map(read).collect::<Result<_>>()?,
            )),
        }
    };
    // `None` for a literal, aligned with `plan.keys`
    let key_sources = plan
        .keys
        .iter()
        .map(|binding| match binding {
            Binding::Attribute(input) => source(input).map(Some),
            Binding::Literal(_) => Ok(None),
        })
        .collect::<Result<Vec<_>>>()?;
    let point_source = plan.point.map(source).transpose()?;
    // Regex scratch space, allocated once per batch; aligned with `plan.keys`.
    let mut key_extractors: Vec<Option<Extractor>> = plan
        .keys
        .iter()
        .map(|binding| match binding {
            Binding::Attribute(input) => Extractor::new(input),
            Binding::Literal(_) => None,
        })
        .collect();
    let mut point_extractor = plan.point.and_then(Extractor::new);

    let candidates = attr_input::candidates(
        key_sources.iter().flatten().chain(point_source.as_ref()),
        is_eligible,
    );
    let mut converted = Vec::with_capacity(plan.keys.len());
    // One list per output, filled in candidate order and handed to `writes`
    // after the loop: a hit is a push, not a search for its destination.
    let mut found_values: Vec<Values> = plan
        .outputs
        .iter()
        .map(|(_, _, ty)| Values::new(*ty))
        .collect();

    for parent in candidates {
        converted.clear();
        let mut usable = true;
        for ((binding, source), extractor) in
            plan.keys.iter().zip(&key_sources).zip(&mut key_extractors)
        {
            let part = match (binding, source) {
                (Binding::Literal(key), _) => {
                    Some(attr_input::Converted::Borrowed(key_cell(key.as_ref())))
                }
                (Binding::Attribute(input), Some(source)) => source
                    .get(parent)
                    .and_then(|v| key_part(v, input.cast, extractor.as_mut())),
                (Binding::Attribute(_), None) => None,
            };
            match part {
                Some(part) => converted.push(part),
                None => {
                    usable = false;
                    break;
                }
            }
        }
        let point = match &point_source {
            Some(source) if usable => match source
                .get(parent)
                .and_then(|v| probe_address(v, point_extractor.as_mut()))
            {
                Some(point) => Some(point),
                None => {
                    usable = false;
                    None
                }
            },
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
            Some(addr) => Lookup::Prefix {
                partition: &parts,
                addr,
            },
            None => Lookup::Hash(&parts),
        };
        // A miss is tallied by `lookup` itself and leaves every destination alone.
        let Some(found) = read.lookup(probe) else {
            continue;
        };
        for ((_, output, _), values) in plan.outputs.iter().zip(&mut found_values) {
            let cell = match output {
                Output::Column(column) => found.row.cell(*column),
                Output::MatchedPrefix => {
                    if let Some(net) = found.prefix {
                        // a v4 CIDR always fits `SmolStr`'s inline buffer; only
                        // a v6 one past 23 characters allocates
                        values.insert_text(parent, format_smolstr!("{net}"));
                    }
                    continue;
                }
            };
            if cell.is_null() {
                continue;
            }
            values.insert(parent, cell);
        }
    }
    let table = writes.table(&rule.table);
    for ((destination, ..), values) in plan.outputs.into_iter().zip(found_values) {
        put(table, destination, values);
    }
    Ok(())
}

/// A key part as a cell, so a literal joins the converted attribute values.
fn key_cell(key: KeyRef<'_>) -> CellRef<'_> {
    match key {
        KeyRef::Bool(v) => CellRef::Bool(v),
        KeyRef::Integer(v) => CellRef::Integer(v),
        KeyRef::String(v) => CellRef::String(v),
        KeyRef::Ip(v) => CellRef::Ip(v),
    }
}

/// A binding's regex extraction as executed: the capture scratch space is
/// allocated once per batch, and the extracted text is a sub-slice of the
/// attribute value, so extraction costs no allocation per record.
pub(super) struct Extractor<'a> {
    regex: &'a Regex,
    group: usize,
    locations: CaptureLocations,
}

impl<'a> Extractor<'a> {
    pub(super) fn new(input: &'a AttributeLookupInput) -> Option<Self> {
        let extract = input.key.extract()?;
        Some(Extractor {
            regex: &extract.pattern.0,
            group: extract.group,
            locations: extract.pattern.0.capture_locations(),
        })
    }

    /// The configured group's text, or `None` for a non-string value, no
    /// match, or an unmatched or empty group.
    fn extract<'v>(&mut self, value: &'v Value<'_>) -> Option<&'v str> {
        let Value::String(text) = value else {
            return None;
        };
        let text = text.as_str();
        self.regex.captures_read(&mut self.locations, text)?;
        let (start, end) = self.locations.get(self.group)?;
        (start < end).then(|| &text[start..end])
    }
}

/// Convert an attribute value into one probe key part.
///
/// Without a cast the value must already be the key column's type: the index
/// compares variants, so `"42"` never matches an integer key. Strings go
/// through [`CellRef::parse`], the parser the table was built with, so probe
/// and build agree — including IPv4-mapped IPv6. With an extractor, the
/// extracted text is what gets cast.
fn key_part<'a>(
    value: &'a Value<'_>,
    cast: Option<AttributeLookupCast>,
    extractor: Option<&mut Extractor<'_>>,
) -> Option<attr_input::Converted<'a>> {
    if let Some(extractor) = extractor {
        let text = extractor.extract(value)?;
        return attr_input::convert_cell(CellRef::String(text), cast.map(cast_type));
    }
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

/// The address a prefix table is probed with. Always read as an IP — OTAP has
/// no IP value, so it arrives as text (or the bare-integer form `parse_ip`
/// also accepts) — whatever `cast` says, which plan time has already pinned
/// to `ip` or unset.
pub(super) fn probe_address(
    value: &Value<'_>,
    extractor: Option<&mut Extractor<'_>>,
) -> Option<IpAddr> {
    match key_part(value, Some(AttributeLookupCast::Ip), extractor)?.as_ref() {
        CellRef::Ip(addr) => Some(addr),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::attr_input::Parent;
    use super::super::attr_write::{TableWrites, apply_table, attrs_payload_type};
    use super::*;
    use arrow::array::{AsArray, RecordBatch, UInt16Array};
    use arrow::datatypes::{DataType, Field, Schema, UInt16Type};
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value as AnyVal;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, KeyValue};
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource;
    use otel_arrow_dfe_pdata::schema::consts;
    use patrolli::cache::lookup_table::mutable::Upsert;
    use patrolli::config::processor::AttributeTable;
    use patrolli::config::processor::attribute_lookup::AttributeLookup;
    use patrolli::payload::attr::{AttrValueCols, StrCol};
    use serde_json::{Value as Json, json};
    use std::collections::BTreeMap;
    use std::sync::Arc;
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

    /// A processor-fed prefix table keyed on its `network` column. `columns`
    /// includes it, but the index consumes it, so `rows` supplies only the
    /// materialized cells beside each CIDR.
    fn prefix_table(
        cache: &cache::Cache,
        partition_columns: &[&str],
        columns: &[(&str, lt::ValueType)],
        rows: &[(&[CellRef<'_>], &str)],
    ) -> Uuid {
        let config = lt::LookupTable {
            id: Uuid::now_v7(),
            name: "prefix".into(),
            source: processor_source(columns),
            key: lt::Key::Prefix(lt::Prefix {
                prefix_column: "network".into(),
                partition_columns: partition_columns.iter().map(|c| c.to_string()).collect(),
            }),
        };
        let table = cache.lookup_table.insert(&config).unwrap();
        let mutable = table.as_mutable().unwrap();
        for (cells, cidr) in rows {
            let outcome = mutable
                .table()
                .upsert_prefix(cells, cidr.parse().unwrap())
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
        attribute_lookup(pl, cfg, Some(Uuid::now_v7()), cache)
    }

    /// Runs with a known node id so the per-processor counters can be read back.
    fn run_counted(
        pl: Payload,
        cfg: &AttributeLookup,
        cache: &cache::Cache,
        table: Uuid,
    ) -> patrolli::common::internal_metrics::LookupTableProcessor {
        let node = Uuid::now_v7();
        let _ = attribute_lookup(pl, cfg, Some(node), cache);
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
            let _ = attribute_lookup(pl.clone(), &cfg, Some(node), &cache);
            let ((_, errored), allocations) = super::super::test_alloc::measure(|| {
                attribute_lookup(pl, &cfg, Some(node), &cache)
            });
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

    /// The processors push each parent once and in ascending order, which is
    /// what the merge needs. Values that arrive otherwise are put in order
    /// here, the later one for a parent winning as it would in a map.
    #[test]
    fn values_out_of_order_or_repeated_are_settled_before_the_merge() {
        let mut pl = logs(
            vec![],
            vec![vec![kv("msg", s("x"))], vec![kv("msg", s("y"))]],
        );
        let mut values = Values::new(lt::ValueType::String);
        values.insert(Parent::Existing(1), CellRef::String("first"));
        values.insert(Parent::Existing(0), CellRef::String("jkt"));
        values.insert(Parent::Existing(1), CellRef::String("sby"));
        let keys = BTreeMap::from([(SmolStr::new("site"), values)]);
        apply_table(&mut pl, &AttributeTable::Record, &keys, false).unwrap();
        let sites: Vec<_> = record_attrs(&pl)
            .into_iter()
            .filter(|(_, key, _)| key == "site")
            .collect();
        assert_eq!(
            sites,
            vec![
                (0, "site".into(), "jkt".into()),
                (1, "site".into(), "sby".into())
            ]
        );
    }

    /// A hit used to cost one heap allocation per record, `Values::insert`
    /// taking an owned `String` of every matched cell; `SmolStr` keeps those
    /// inline. Asserted as a growth rate, so the fixed per-batch overhead may
    /// change but the slope must stay near zero — one-per-record reads ~900.
    #[test]
    fn enriching_a_record_does_not_allocate_per_record() {
        let cache = cache::Cache::new();
        let hash = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[&[CellRef::String("h"), CellRef::String("dc1")]],
        );
        let hash_cfg = config(
            json!({"lookups":[{"lookup_table":hash,"table":"record","match":{"keys":{"host":{"key":"host"}}},"output":{"site":"site"}}]}),
        );
        // A prefix probe, emitting the matched prefix as well. Only v4: a v4
        // CIDR always fits `SmolStr`'s 23-byte inline buffer, while a long v6
        // one (`2001:db8:1234:5678::/64`) allocates once per record by design.
        let prefix = prefix_table(
            &cache,
            &[],
            &[
                ("network", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[
                (&[CellRef::String("dc0")], "10.0.0.0/8"),
                (&[CellRef::String("dc1")], "10.1.0.0/16"),
            ],
        );
        let prefix_cfg = config(
            json!({"lookups":[{"lookup_table":prefix,"table":"record","match":{"point":{"key":"host"}},"output":{"site":"site","net":"network"}}]}),
        );
        for (cfg, host) in [(&hash_cfg, "h"), (&prefix_cfg, "10.1.2.3")] {
            let node = Uuid::now_v7();
            let measure = |records: usize| {
                let pl = logs(
                    vec![],
                    (0..records).map(|_| vec![kv("host", s(host))]).collect(),
                );
                // once to warm every lazily grown map, then measure a clean run
                let _ = attribute_lookup(pl.clone(), cfg, Some(node), &cache);
                let ((pl, errored), allocations) = super::super::test_alloc::measure(|| {
                    attribute_lookup(pl, cfg, Some(node), &cache)
                });
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
                "900 more records must not cost ~900 more allocations ({host}): {small} -> {large}"
            );
        }
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
    fn unpartitioned_and_partitioned_prefixes_both_probe() {
        for partitioned in [false, true] {
            let cache = cache::Cache::new();
            let columns: Vec<(&str, lt::ValueType)> = if partitioned {
                vec![
                    ("network", lt::ValueType::String),
                    ("site", lt::ValueType::String),
                    ("zone", lt::ValueType::String),
                ]
            } else {
                vec![
                    ("network", lt::ValueType::String),
                    ("zone", lt::ValueType::String),
                ]
            };
            let partition: &[&str] = if partitioned { &["site"] } else { &[] };
            let cells: Vec<CellRef<'_>> = if partitioned {
                vec![CellRef::String("jkt"), CellRef::String("dmz")]
            } else {
                vec![CellRef::String("dmz")]
            };
            let id = prefix_table(&cache, partition, &columns, &[(&cells, "10.0.0.0/8")]);

            let mut attrs = vec![kv("addr", s("10.1.2.3"))];
            let mut keys = json!({});
            if partitioned {
                attrs.push(kv("site", s("jkt")));
                keys = json!({"site": {"key": "site"}});
            }
            let pl = logs(vec![], vec![attrs]);
            let cfg = config(json!({"lookups": [{
                "lookup_table": id,
                "table": "record",
                "match": {"keys": keys, "point": {"key": "addr"}},
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
    fn a_key_list_takes_the_first_present_attribute() {
        let cache = cache::Cache::new();
        let id = prefix_table(
            &cache,
            &["router"],
            &[
                ("network", lt::ValueType::String),
                ("router", lt::ValueType::Ip),
                ("asn", lt::ValueType::Integer),
            ],
            &[
                (
                    &[
                        CellRef::Ip("192.0.2.1".parse().unwrap()),
                        CellRef::Integer(1),
                    ],
                    "10.0.0.0/8",
                ),
                (
                    &[
                        CellRef::Ip("192.0.2.2".parse().unwrap()),
                        CellRef::Integer(2),
                    ],
                    "10.0.0.0/8",
                ),
                (
                    &[
                        CellRef::Ip("192.0.2.2".parse().unwrap()),
                        CellRef::Integer(6),
                    ],
                    "2001:db8::/32",
                ),
            ],
        );
        let pl = logs(
            vec![],
            vec![
                // mapped: the first key wins over the exporter
                vec![
                    kv("bgp.router", s("192.0.2.2")),
                    kv("source.address", s("192.0.2.1")),
                    kv("dst4", s("10.1.1.1")),
                ],
                // unmapped: falls back to the exporter
                vec![
                    kv("source.address", s("192.0.2.1")),
                    kv("dst4", s("10.1.1.1")),
                ],
                // a v6 flow, through the point's second key
                vec![
                    kv("source.address", s("192.0.2.2")),
                    kv("dst6", s("2001:db8::1")),
                ],
                // the first key is present but not an address: an error, and
                // the exporter is not tried
                vec![
                    kv("bgp.router", s("not-an-ip")),
                    kv("source.address", s("192.0.2.1")),
                    kv("dst4", s("10.1.1.1")),
                ],
            ],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id,
            "table": "record",
            "match": {
                "keys": {"router": {"key": ["bgp.router", "source.address"], "cast": "ip"}},
                "point": {"key": ["dst4", "dst6"]},
            },
            "output": {"dst.as": "asn"},
        }]}));

        let (out, errored) = run(pl.clone(), &cfg, &cache);
        assert!(!errored);
        let attrs = record_attrs(&out);
        let as_of = |parent| {
            attrs
                .iter()
                .find(|(p, k, _)| *p == parent && k == "dst.as")
                .map(|(_, _, v)| v.clone())
        };
        assert_eq!(as_of(0).as_deref(), Some("i2"), "{attrs:?}");
        assert_eq!(as_of(1).as_deref(), Some("i1"), "{attrs:?}");
        assert_eq!(as_of(2).as_deref(), Some("i6"), "{attrs:?}");
        assert_eq!(as_of(3), None, "{attrs:?}");

        let m = run_counted(pl, &cfg, &cache, id);
        assert_eq!(m.lookups_hit, 3);
        assert_eq!(m.lookups_errored, 1);
    }

    /// The value of `key` on each record parent, in parent order.
    fn values_of(pl: &Payload, key: &str) -> Vec<(u16, String)> {
        record_attrs(pl)
            .into_iter()
            .filter(|(_, k, _)| k == key)
            .map(|(parent, _, value)| (parent, value))
            .collect()
    }

    #[test]
    fn an_extracted_group_probes_instead_of_the_whole_value() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["ifname"],
            &[
                ("ifname", lt::ValueType::String),
                ("alias", lt::ValueType::String),
            ],
            &[&[CellRef::String("ae21"), CellRef::String("uplink")]],
        );
        let pl = logs(
            vec![],
            vec![
                vec![kv("interface.name", s("ae21.0"))], // unit stripped -> hit
                vec![kv("interface.name", s("ae21"))],   // no unit: no match -> key error
                vec![kv("interface.name", s("ae22.0"))], // extracted, no row -> miss
                vec![kv("interface.name", int(21))],     // not a string -> key error
                // the first key is present but does not match: an error, and
                // the second key is not tried
                vec![kv("if.name", s("ae21")), kv("interface.name", s("ae21.0"))],
            ],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"ifname": {"key": {
                "from": ["if.name", "interface.name"],
                "pattern": r"^(.+)\.\d+$",
            }}}},
            "output": {"interface.alias": "alias"},
        }]}));

        let (out, errored) = run(pl.clone(), &cfg, &cache);
        assert!(!errored);
        assert_eq!(
            values_of(&out, "interface.alias"),
            vec![(0, "uplink".into())]
        );
        let m = run_counted(pl, &cfg, &cache, id);
        assert_eq!(m.lookups_hit, 1);
        assert_eq!(m.lookups_missed, 1);
        assert_eq!(m.lookups_errored, 3);
    }

    #[test]
    fn extracted_text_is_cast_and_a_point_can_be_extracted_too() {
        let cache = cache::Cache::new();
        let hash = hash_table(
            &cache,
            &["idx"],
            &[
                ("idx", lt::ValueType::Integer),
                ("name", lt::ValueType::String),
            ],
            &[&[CellRef::Integer(21), CellRef::String("ge-0/0/21")]],
        );
        let prefix = prefix_table(
            &cache,
            &[],
            &[
                ("network", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
            &[(&[CellRef::String("jkt")], "10.0.0.0/8")],
        );
        let pl = logs(
            vec![],
            vec![vec![kv("port", s("Gi0/21")), kv("peer", s("10.1.1.1:443"))]],
        );
        let cfg = config(json!({"lookups": [
            {
                "lookup_table": hash, "table": "record",
                "match": {"keys": {"idx": {
                    "key": {"from": "port", "pattern": r"(\d+)$"},
                    "cast": "integer",
                }}},
                "output": {"port.name": "name"},
            },
            {
                "lookup_table": prefix, "table": "record",
                "match": {"point": {"key": {"from": "peer", "pattern": "^[^:]+", "group": 0}}},
                "output": {"peer.site": "site"},
            },
        ]}));

        let (out, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert_eq!(values_of(&out, "port.name"), vec![(0, "ge-0/0/21".into())]);
        assert_eq!(values_of(&out, "peer.site"), vec![(0, "jkt".into())]);
    }

    #[test]
    fn an_uncast_extraction_into_a_non_string_column_is_a_plan_error() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["idx"],
            &[
                ("idx", lt::ValueType::Integer),
                ("name", lt::ValueType::String),
            ],
            &[&[CellRef::Integer(21), CellRef::String("ge-0/0/21")]],
        );
        let pl = logs(vec![], vec![vec![kv("port", s("Gi0/21"))]]);
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"idx": {"key": {"from": "port", "pattern": r"(\d+)$"}}}},
            "output": {"port.name": "name"},
        }]}));
        let (out, errored) = run(pl.clone(), &cfg, &cache);
        assert!(errored);
        assert_eq!(record_attrs(&out), record_attrs(&pl));
    }

    #[test]
    fn extraction_adds_no_per_candidate_allocations() {
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
        let pl = logs(
            vec![],
            (0..1000)
                .map(|_| vec![kv("host", s("a-long-host-name-past-inline.example.net"))])
                .collect(),
        );
        let node = Uuid::now_v7();
        let counts: Vec<usize> = [
            json!("host"),
            json!({"from": "host", "pattern": r"^([^.]+)\."}),
        ]
        .into_iter()
        .map(|key| {
            let cfg = config(json!({"lookups": [{
                "lookup_table": id, "table": "record",
                "match": {"keys": {"host": {"key": key}}},
                "output": {"site": "site"},
            }]}));
            let _ = attribute_lookup(pl.clone(), &cfg, Some(node), &cache);
            let ((_, errored), allocations) = super::super::test_alloc::measure(|| {
                attribute_lookup(pl.clone(), &cfg, Some(node), &cache)
            });
            assert!(!errored);
            allocations.calls
        })
        .collect();
        // the capture scratch space, once per batch, and nothing per record
        assert!(counts[1] <= counts[0] + 4, "{counts:?}");
    }

    #[test]
    fn the_longest_prefix_wins_and_can_be_output() {
        let cache = cache::Cache::new();
        let id = prefix_table(
            &cache,
            &[],
            &[
                ("network", lt::ValueType::String),
                ("asn", lt::ValueType::Integer),
            ],
            &[
                (&[CellRef::Integer(8)], "10.0.0.0/8"),
                (&[CellRef::Integer(24)], "10.1.2.0/24"),
                (&[CellRef::Integer(16)], "10.1.0.0/16"),
                (&[CellRef::Integer(64)], "2001:db8:1234:5678::/64"),
            ],
        );
        let pl = logs(
            vec![],
            vec![
                vec![kv("dst", s("10.1.2.3"))],
                vec![kv("dst", s("10.1.9.9"))],
                vec![kv("dst", s("10.9.9.9"))],
                vec![kv("dst", s("2001:db8:1234:5678::1"))],
                vec![kv("dst", s("11.0.0.1"))],
            ],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id,
            "table": "record",
            "match": {"point": {"key": "dst"}},
            "output": {"dst.asn": "asn", "dst.prefix": "network"},
        }]}));

        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        let attrs = record_attrs(&pl);
        for (parent, asn, prefix) in [
            (0, "i24", "10.1.2.0/24"),
            (1, "i16", "10.1.0.0/16"),
            (2, "i8", "10.0.0.0/8"),
            (3, "i64", "2001:db8:1234:5678::/64"),
        ] {
            assert!(
                attrs.contains(&(parent, "dst.asn".into(), asn.into())),
                "{parent}: {attrs:?}"
            );
            assert!(
                attrs.contains(&(parent, "dst.prefix".into(), prefix.into())),
                "{parent}: {attrs:?}"
            );
        }
        assert!(
            !attrs
                .iter()
                .any(|(p, k, _)| *p == 4 && k.starts_with("dst.")),
            "an uncovered address must not be enriched: {attrs:?}"
        );
    }

    #[test]
    fn a_prefix_probe_canonicalises_the_v4_mapped_form() {
        let cache = cache::Cache::new();
        let id = prefix_table(
            &cache,
            &[],
            &[
                ("network", lt::ValueType::String),
                ("country", lt::ValueType::String),
            ],
            &[(&[CellRef::String("ID")], "10.0.0.0/24")],
        );
        // The mapped form must fold onto v4, or the probe would look in the v6
        // trie and miss.
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
            "an uncovered address must not be enriched: {attrs:?}"
        );
    }

    /// The point of a prefix table is always an address: a cast may be left out
    /// or spelled `ip`, and anything else is a plan error.
    #[test]
    fn a_prefix_point_cast_must_be_ip_or_absent() {
        let cache = cache::Cache::new();
        let id = prefix_table(
            &cache,
            &[],
            &[
                ("network", lt::ValueType::String),
                ("country", lt::ValueType::String),
            ],
            &[(&[CellRef::String("ID")], "10.0.0.0/24")],
        );
        for (cast, want_error) in [
            (None, false),
            (Some("ip"), false),
            (Some("string"), true),
            (Some("integer"), true),
        ] {
            let mut point = json!({"key": "src"});
            if let Some(cast) = cast {
                point["cast"] = json!(cast);
            }
            // unvalidated, so the plan-time check is what is exercised;
            // config validation rejects the same casts earlier
            let cfg: AttributeLookup = serde_json::from_value(json!({"lookups": [{
                "lookup_table": id, "table": "record",
                "match": {"point": point},
                "output": {"country": "country"},
            }]}))
            .unwrap();
            assert_eq!(cfg.validate().is_err(), want_error, "{cast:?}");
            let (pl, errored) = run(
                logs(vec![], vec![vec![kv("src", s("10.0.0.1"))]]),
                &cfg,
                &cache,
            );
            assert_eq!(errored, want_error, "{cast:?}");
            assert_eq!(
                record_attrs(&pl).contains(&(0, "country".into(), "ID".into())),
                !want_error,
                "{cast:?}"
            );
        }

        // and a prefix table needs a point at all
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"keys": {"country": {"key": "c"}}},
            "output": {"country": "country"},
        }]}));
        let (_, errored) = run(logs(vec![], vec![vec![kv("c", s("ID"))]]), &cfg, &cache);
        assert!(errored);
    }

    /// An unparseable address is a key error for that record, not a miss and
    /// not a rule failure.
    #[test]
    fn an_unparseable_point_is_a_key_error() {
        let cache = cache::Cache::new();
        let id = prefix_table(
            &cache,
            &[],
            &[
                ("network", lt::ValueType::String),
                ("country", lt::ValueType::String),
            ],
            &[(&[CellRef::String("ID")], "10.0.0.0/24")],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"point": {"key": "src"}},
            "output": {"country": "country"},
        }]}));
        let node = Uuid::now_v7();
        let pl = logs(
            vec![],
            vec![
                vec![kv("src", s("not-an-ip"))],
                vec![kv("src", s("10.0.0.1"))],
            ],
        );
        let (pl, errored) = attribute_lookup(pl, &cfg, Some(node), &cache);
        assert!(!errored);
        assert!(record_attrs(&pl).contains(&(1, "country".into(), "ID".into())));
        let table = cache.lookup_table.get(&id).unwrap();
        let usage = table.meta().metrics.processor_snapshots();
        let usage = usage.iter().find(|p| p.id == node).unwrap();
        assert_eq!(usage.lookups_errored, 1);
        assert_eq!(usage.lookups_hit, 1);
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

    /// The check lives in `bound`, which both key kinds go through, so a prefix
    /// table's partition columns are covered by the same rule as a hash key's.
    #[test]
    fn a_prefix_partition_column_is_checked_the_same_way_as_a_hash_key() {
        let cache = cache::Cache::new();
        let id = prefix_table(
            &cache,
            &["ip"],
            &[
                ("ip", lt::ValueType::Ip),
                ("network", lt::ValueType::String),
                ("country", lt::ValueType::String),
            ],
            &[(
                &[
                    CellRef::Ip("10.0.0.1".parse().unwrap()),
                    CellRef::String("ID"),
                ],
                "192.0.2.0/24",
            )],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {
                "keys": {"ip": {"key": "ip"}},
                "point": {"key": "n"},
            },
            "output": {"country": "country"},
        }]}));
        let pl = logs(
            vec![],
            vec![vec![kv("ip", s("10.0.0.1")), kv("n", s("192.0.2.9"))]],
        );
        let (_, errored) = run(pl, &cfg, &cache);
        assert!(errored, "an uncast ip partition column must fail the rule");
    }

    #[test]
    fn a_literal_vrf_probes_only_the_global_routes_of_the_exporter() {
        use patrolli::cache::lookup_table::bgp::{PathSegment, RouteRow, RouteText};
        let cache = cache::Cache::new();
        let table: lt::LookupTable = serde_json::from_value(json!({
            "id": Uuid::now_v7(),
            "name": "routes",
            "source": {"type": "bgp", "listener": Uuid::now_v7()},
        }))
        .unwrap();
        cache.lookup_table.insert(&table).unwrap();
        let routes = cache.lookup_table.route_table(&table.id).unwrap();
        let mut text = RouteText::default();
        let global = [PathSegment::Sequence(&[64500, 64501])];
        let vrf = [PathSegment::Sequence(&[64999])];
        for (vrf, path) in [("global", &global[..]), ("64500:7", &vrf[..])] {
            let row = RouteRow {
                router: "192.0.2.1".parse().unwrap(),
                vrf,
                prefix: "198.51.100.0/24".parse().unwrap(),
                next_hop: None,
                as_path: path,
                communities: &[],
                large_communities: &[],
                local_pref: None,
                med: None,
                path_id: None,
            };
            assert_eq!(routes.upsert(&row, &mut text).unwrap(), Upsert::Inserted);
        }

        let pl = logs(
            vec![],
            vec![
                vec![kv("exporter", s("192.0.2.1")), kv("dst", s("198.51.100.9"))],
                // another router: its partition is empty
                vec![kv("exporter", s("192.0.2.2")), kv("dst", s("198.51.100.9"))],
            ],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": table.id, "table": "record",
            "match": {
                "keys": {"router": {"key": "exporter", "cast": "ip"}},
                "literals": {"vrf": "global"},
                "point": {"key": "dst"},
            },
            "output": {"dst_as": "origin_as", "dst_path": "as_path", "dst_net": "prefix"},
        }]}));
        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        let attrs = record_attrs(&pl);
        assert!(
            attrs.contains(&(0, "dst_as".into(), "i64501".into())),
            "{attrs:?}"
        );
        assert!(attrs.contains(&(0, "dst_path".into(), "64500 64501".into())));
        assert!(attrs.contains(&(0, "dst_net".into(), "198.51.100.0/24".into())));
        assert!(
            !attrs
                .iter()
                .any(|(parent, key, _)| *parent == 1 && key == "dst_as"),
            "another router's flow is not enriched: {attrs:?}"
        );
    }

    #[test]
    fn an_ip_literal_needs_no_cast() {
        let cache = cache::Cache::new();
        let id = prefix_table(
            &cache,
            &["ip"],
            &[
                ("ip", lt::ValueType::Ip),
                ("network", lt::ValueType::String),
                ("country", lt::ValueType::String),
            ],
            &[(
                &[
                    CellRef::Ip("10.0.0.1".parse().unwrap()),
                    CellRef::String("ID"),
                ],
                "192.0.2.0/24",
            )],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"literals": {"ip": "::ffff:10.0.0.1"}, "point": {"key": "n"}},
            "output": {"country": "country"},
        }]}));
        let pl = logs(vec![], vec![vec![kv("n", s("192.0.2.9"))]]);
        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert!(
            record_attrs(&pl).contains(&(0, "country".into(), "ID".into())),
            "parsed like a stored row, so v4-mapped matches v4"
        );
    }

    #[test]
    fn a_bad_literal_or_a_wrong_binding_count_fails_the_plan() {
        let cache = cache::Cache::new();
        let id = prefix_table(
            &cache,
            &["asn"],
            &[
                ("asn", lt::ValueType::Integer),
                ("network", lt::ValueType::String),
                ("name", lt::ValueType::String),
            ],
            &[(
                &[CellRef::Integer(64500), CellRef::String("x")],
                "192.0.2.0/24",
            )],
        );
        let rule = |by: Json| {
            let cfg = config(json!({"lookups": [{
                "lookup_table": id, "table": "record",
                "match": by,
                "output": {"name": "name"},
            }]}));
            let pl = logs(vec![], vec![vec![kv("n", s("192.0.2.9"))]]);
            let (pl, errored) = run(pl, &cfg, &cache);
            (errored, record_attrs(&pl))
        };
        let (errored, attrs) = rule(json!({"literals": {"asn": 64500}, "point": {"key": "n"}}));
        assert!(!errored);
        assert!(attrs.contains(&(0, "name".into(), "x".into())));
        let (errored, _) = rule(json!({"literals": {"asn": "64500"}, "point": {"key": "n"}}));
        assert!(
            !errored,
            "quoted or not, the text parses as the column's type"
        );

        let (errored, _) = rule(json!({"literals": {"asn": "AS64500"}, "point": {"key": "n"}}));
        assert!(errored, "a literal that does not parse");
        let (errored, _) = rule(json!({
            "keys": {"asn": {"key": "a"}},
            "literals": {"other": 1},
            "point": {"key": "n"},
        }));
        assert!(errored, "literals count toward the bindings");
    }

    #[test]
    fn a_hash_key_bound_only_by_literals_is_a_plan_error() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["sid"],
            &[
                ("sid", lt::ValueType::String),
                ("name", lt::ValueType::String),
            ],
            &[&[CellRef::String("a"), CellRef::String("x")]],
        );
        let cfg = config(json!({"lookups": [{
            "lookup_table": id, "table": "record",
            "match": {"literals": {"sid": "a"}},
            "output": {"name": "name"},
        }]}));
        let pl = logs(vec![], vec![vec![kv("sid", s("a"))]]);
        let (_, errored) = run(pl, &cfg, &cache);
        assert!(errored);
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
