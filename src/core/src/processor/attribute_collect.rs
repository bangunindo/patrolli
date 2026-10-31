// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The `attribute_collect` processor: complete rows harvested out of incoming
//! attributes and upserted into mutable lookup tables. Config contract on
//! [`crate::config::processor::attribute_collect`]; this module is its execution.
//!
//! The mirror of [`super::attribute_lookup`] — same guards, same parents, same
//! parser — differing only in direction: a parent becomes a table row rather
//! than a probe key, and the payload is never touched. It resolves the target per rule per batch for the reason given
//! there.
//!
//! Nothing here can cost telemetry; a rule that cannot be planned, an
//! unbuildable row and a row rejected for capacity differ only in which counter
//! moves. Rows are complete by construction: a missing input, a null key or
//! bound, or a failed cast skips the whole candidate, because the table
//! replaces a key's row outright and a partial write would drop columns.

use super::attr_input;
use super::attr_input::Parent;
use crate::cache;
use crate::cache::lookup_table::cell::{CellRef, RangePoint};
use crate::cache::lookup_table::mutable::Mutable;
use crate::cache::lookup_table::schema::{ColumnIdx, KeyPlan};
use crate::config::lookup_table as lt;
use crate::config::processor::AttributeTable;
use crate::config::processor::attribute_collect::{
    AttributeCollect, AttributeCollectCast, AttributeCollectInput, AttributeCollectPivot,
    AttributeCollectRule, NameFormat,
};
use crate::payload::{self, Payload, Value, attr};
use anyhow::{Result, anyhow, bail};
use arrow::array::BooleanArray;
use regex::{CaptureLocations, Regex};
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;
use smol_str::SmolStr;
use tracing::{trace, warn};
use uuid::Uuid;

/// Collect from `pl` and report whether any rule was skipped by an error. `pl`
/// comes back unchanged, and there is no failing return.
pub(super) fn attribute_collect(
    pl: payload::Payload,
    config: &AttributeCollect,
    node_id: Uuid,
    cache: &cache::Cache,
) -> (payload::Payload, bool) {
    if pl.is_empty() {
        return (pl, false);
    }
    let mut errored = false;

    // Evaluated once and shared by every rule, as in `attribute_lookup`.
    let shared = match &config.when {
        None => None,
        Some(filter) => match attr::attribute_filter_mask(&pl, filter) {
            Ok(mask) => Some(mask),
            Err(e) => {
                warn!(error = %format!("{e:#}"), "shared `when` failed, skipping collection");
                return (pl, true);
            }
        },
    };

    for (i, rule) in config.collections.iter().enumerate() {
        if let Err(e) = run_rule(&pl, rule, shared.as_ref(), node_id, cache) {
            errored = true;
            warn!(
                error = %format!("{e:#}"),
                collection = i,
                lookup_table_id = %rule.lookup_table,
                "collection skipped"
            );
        }
    }
    (pl, errored)
}

// ---- one rule ----

/// A rule's config resolved against its target table.
struct Plan<'a> {
    /// One binding per materialized column, in `ColumnIdx` order — which is the
    /// order [`Mutable::upsert_hash`] reads its cells in.
    columns: Vec<(Binding<'a>, lt::ValueType)>,
    /// The materialized columns the key is taken from: a hash table's key
    /// columns, or a range table's partition columns.
    key_columns: Box<[ColumnIdx]>,
    /// The interval bindings, `Some` exactly when the table is range-keyed.
    bounds: Option<Bounds<'a>>,
    /// The distinct normalized element names the pivoted bindings ask for;
    /// [`Binding::Pivot::slot`] indexes into it.
    pivot_names: Vec<&'a str>,
}

/// Where one column's cell comes from.
#[derive(Clone, Copy)]
enum Binding<'a> {
    /// An exact attribute key, one value per parent.
    Key(&'a AttributeCollectInput),
    /// The `field` of the pivot element whose normalized name is
    /// `plan.pivot_names[slot]`.
    Pivot {
        slot: usize,
        field: &'a str,
        cast: Option<AttributeCollectCast>,
    },
}

impl Binding<'_> {
    fn cast(&self) -> Option<AttributeCollectCast> {
        match self {
            Binding::Key(input) => input.cast,
            Binding::Pivot { cast, .. } => *cast,
        }
    }
}

/// The two bound columns of a range table, plus the inclusivity and range type
/// that belong to the table rather than to this processor.
struct Bounds<'a> {
    start: Binding<'a>,
    end: Binding<'a>,
    start_inclusive: bool,
    end_inclusive: bool,
    range_type: lt::RangeType,
}

impl<'a> Plan<'a> {
    fn resolve(
        table: &Mutable,
        config: &lt::LookupTable,
        rule: &'a AttributeCollectRule,
    ) -> Result<Self> {
        let schema = table.schema();
        // Bound columns are consumed by the index and never materialized, so
        // their names come off the table's own key config, not `schema`.
        let range = match &config.key {
            lt::Key::Hash(_) => None,
            lt::Key::Range(range) => Some(range),
        };

        // Every declared column must be bound exactly once. `columns` is a map,
        // so only "missing" and "unknown" are left to check.
        let declared: Vec<&str> = schema
            .names()
            .iter()
            .map(SmolStr::as_str)
            .chain(
                range
                    .iter()
                    .flat_map(|r| [r.start_column.as_str(), r.end_column.as_str()]),
            )
            .collect();
        let mut pivot_names: Vec<&'a str> = Vec::new();
        let mut binding = |name: &str| -> Result<Binding<'a>> {
            let (column, bound) = rule
                .columns
                .get_key_value(name)
                .ok_or_else(|| anyhow!("columns has no binding for table column {name:?}"))?;
            Ok(match &bound.0 {
                Some(input) if input.key.is_some() => Binding::Key(input),
                // A `null` binding or one without a key: config validation
                // guarantees the rule has a pivot, but the plan is what the
                // processor trusts.
                input => {
                    let pivot = rule.pivot.as_ref().ok_or_else(|| {
                        anyhow!("columns[{name:?}] is pivoted but the rule has no `pivot`")
                    })?;
                    let input = input.as_ref();
                    let object = input
                        .and_then(|input| input.name.as_deref())
                        .unwrap_or(column.as_str());
                    let slot = match pivot_names.iter().position(|n| *n == object) {
                        Some(slot) => slot,
                        None => {
                            pivot_names.push(object);
                            pivot_names.len() - 1
                        }
                    };
                    Binding::Pivot {
                        slot,
                        field: input
                            .and_then(|input| input.value.as_deref())
                            .unwrap_or(pivot.value.as_str()),
                        cast: input.and_then(|input| input.cast),
                    }
                }
            })
        };
        let unknown: Vec<&str> = rule
            .columns
            .keys()
            .map(String::as_str)
            .filter(|name| !declared.contains(name))
            .collect();
        if !unknown.is_empty() {
            bail!("columns names {unknown:?}, which the table does not declare");
        }

        let key_columns = match table.key_plan() {
            KeyPlan::Hash { columns } => columns.clone(),
            KeyPlan::Range { partition, .. } => partition.clone(),
        };

        let mut columns = Vec::with_capacity(schema.len());
        for idx in 0..schema.len() as ColumnIdx {
            let name = schema.name(idx).expect("index taken from the schema");
            let ty = schema.value_type(idx).expect("index taken from the schema");
            let bound = binding(name)?;
            // Without a cast the attribute must already be `ty`, checked per
            // value; a cast naming anything else is a config mistake.
            if let Some(cast) = bound.cast()
                && cast_type(cast) != ty
            {
                bail!("columns[{name:?}].cast is {cast:?} but the column is {ty:?}");
            }
            // No OTAP IP value, so an ip column always arrives as a string.
            // Without the cast every real address skips its candidate row while
            // null ones still store, so the table fills with nulls and looks
            // alive. `bounds` below enforces the same rule.
            if bound.cast().is_none() && ty == lt::ValueType::Ip {
                bail!("an ip column needs an explicit `cast: ip` on columns[{name:?}]");
            }
            // `KeyRef` has no double variant. A lookup-table misconfiguration,
            // but it surfaces here.
            if ty == lt::ValueType::Double && key_columns.contains(&idx) {
                bail!("key column {name:?} is a double and cannot index a lookup table");
            }
            columns.push((bound, ty));
        }

        let bounds = range
            .map(|range| {
                let want = match range.range_type {
                    lt::RangeType::Ip => AttributeCollectCast::Ip,
                    lt::RangeType::Integer => AttributeCollectCast::Integer,
                };
                let mut bound = |name: &str| -> Result<Binding<'a>> {
                    let bound = binding(name)?;
                    match bound.cast() {
                        Some(cast) if cast != want => bail!(
                            "columns[{name:?}].cast is {cast:?} but the table's range is {:?}",
                            range.range_type
                        ),
                        // No OTAP IP value: an ip bound arrives as a string.
                        None if range.range_type == lt::RangeType::Ip => {
                            bail!("an ip range needs an explicit `cast: ip` on columns[{name:?}]")
                        }
                        _ => {}
                    }
                    Ok(bound)
                };
                Ok::<_, anyhow::Error>(Bounds {
                    start: bound(&range.start_column)?,
                    end: bound(&range.end_column)?,
                    start_inclusive: range.start_inclusive,
                    end_inclusive: range.end_inclusive,
                    range_type: range.range_type,
                })
            })
            .transpose()?;

        Ok(Plan {
            columns,
            key_columns,
            bounds,
            pivot_names,
        })
    }

    /// Every binding of the plan, columns then bounds.
    fn bindings(&self) -> impl Iterator<Item = &Binding<'a>> {
        self.columns.iter().map(|(binding, _)| binding).chain(
            self.bounds
                .iter()
                .flat_map(|bounds| [&bounds.start, &bounds.end]),
        )
    }
}

// ---- pivot ----

/// The pivot array of one rule, read once per batch.
///
/// Names are indexed by *slot* rather than by text so the map never borrows
/// from the name values it was built from, and because the handful of names a
/// rule binds is all that is ever looked up.
struct PivotData<'p> {
    /// `(parent, name slot) -> element index`, only for the names some pivoted
    /// binding asks for. The highest index wins on a duplicate.
    index: FxHashMap<(u16, usize), u32>,
    /// `field -> (parent, element index) -> value`, one map per distinct
    /// element field bound. Nulls are kept, as for exact keys.
    fields: FxHashMap<&'p str, FxHashMap<(u16, u32), Value<'p>>>,
}

impl<'p> PivotData<'p> {
    fn read(
        pl: &'p Payload,
        table: &AttributeTable,
        pivot: &'p AttributeCollectPivot,
        plan: &Plan<'p>,
    ) -> Result<Self> {
        let mut fields: FxHashMap<&'p str, FxHashMap<(u16, u32), Value<'p>>> = FxHashMap::default();
        for binding in plan.bindings() {
            if let Binding::Pivot { field, .. } = binding {
                fields.entry(field).or_default();
            }
        }
        let mut index = FxHashMap::default();
        let Some(batch) = attr_input::attr_batch(pl, table) else {
            return Ok(Self { index, fields });
        };

        let prefix = format!("{}.", pivot.array);
        // A null name names nothing, so nulls are dropped here.
        let names =
            attr::parent_id_element_values(batch, &prefix, &format!(".{}", pivot.name), false)?;
        let mut normalizer = Normalizer::new(pivot.name_format.as_ref());
        for ((parent, element), raw) in &names {
            let Value::String(raw) = raw else {
                continue;
            };
            let Some(name) = normalizer.normalize(raw.as_str()) else {
                continue;
            };
            // A walk carries far more objects than a rule binds; unbound names
            // take no map entry.
            let Some(slot) = plan.pivot_names.iter().position(|n| *n == name) else {
                continue;
            };
            index
                .entry((*parent, slot))
                .and_modify(|current: &mut u32| *current = (*current).max(*element))
                .or_insert(*element);
        }
        for (field, values) in &mut fields {
            *values = attr::parent_id_element_values(batch, &prefix, &format!(".{field}"), true)?;
        }
        Ok(Self { index, fields })
    }

    fn get(&self, parent: Parent, slot: usize, field: &str) -> Option<&Value<'p>> {
        let parent = parent.encoded()?;
        let element = *self.index.get(&(parent, slot))?;
        self.fields.get(field)?.get(&(parent, element))
    }

    /// A parent carrying any bound element is a candidate, like one carrying
    /// any exact-key input.
    fn extend_candidates(
        &self,
        is_eligible: &impl Fn(Parent) -> bool,
        out: &mut FxHashSet<Parent>,
    ) {
        out.extend(
            self.index
                .keys()
                .map(|(parent, _)| Parent::Existing(*parent))
                .filter(|parent| is_eligible(*parent)),
        );
    }
}

/// [`NameFormat`] as executed: the regex scratch space is allocated once per
/// batch, and every format hands back a sub-slice of its input.
enum Normalizer<'a> {
    Verbatim,
    SnmpObject {
        strip_instance: bool,
    },
    Regex {
        regex: &'a Regex,
        group: usize,
        locations: CaptureLocations,
    },
}

impl<'a> Normalizer<'a> {
    fn new(format: Option<&'a NameFormat>) -> Self {
        match format {
            None => Normalizer::Verbatim,
            Some(NameFormat::SnmpObject(format)) => Normalizer::SnmpObject {
                strip_instance: format.strip_instance,
            },
            Some(NameFormat::RegexExtract(format)) => Normalizer::Regex {
                regex: &format.pattern.0,
                group: format.group,
                locations: format.pattern.0.capture_locations(),
            },
        }
    }

    /// The name to match against the bound columns, or `None` when `raw` does
    /// not fit the format. Never empty.
    fn normalize<'s>(&mut self, raw: &'s str) -> Option<&'s str> {
        let name = match self {
            Normalizer::Verbatim => raw,
            Normalizer::SnmpObject { strip_instance } => {
                // `IF-MIB::ifDescr.33`: module, object, instance. A name without
                // a module keeps working; one without a MIB match is a bare
                // numeric OID whose "object" is its first arc.
                let object = raw.rsplit_once("::").map_or(raw, |(_, object)| object);
                if *strip_instance {
                    object.split_once('.').map_or(object, |(object, _)| object)
                } else {
                    object
                }
            }
            Normalizer::Regex {
                regex,
                group,
                locations,
            } => {
                regex.captures_read(locations, raw)?;
                let (start, end) = locations.get(*group)?;
                &raw[start..end]
            }
        };
        (!name.is_empty()).then_some(name)
    }
}

/// One column's inputs for a batch.
enum ColumnSource<'p> {
    Attr(attr_input::Source<'p>),
    Pivot { slot: usize, field: &'p str },
}

impl<'p> ColumnSource<'p> {
    fn get<'s>(
        &'s self,
        parent: Parent,
        pivot: Option<&'s PivotData<'p>>,
    ) -> Option<&'s Value<'p>> {
        match self {
            ColumnSource::Attr(source) => source.get(parent),
            ColumnSource::Pivot { slot, field } => pivot?.get(parent, *slot, field),
        }
    }
}

/// `pl` and `rule` share one lifetime: a column source borrows attribute values
/// from the first and pivot field names from the second.
fn run_rule<'a>(
    pl: &'a Payload,
    rule: &'a AttributeCollectRule,
    shared: Option<&BooleanArray>,
    node_id: Uuid,
    cache: &cache::Cache,
) -> Result<()> {
    let table = cache
        .lookup_table
        .get(&rule.lookup_table)
        .ok_or_else(|| anyhow!("lookup table {} is not configured", rule.lookup_table))?;
    let mutable = table.as_mutable().ok_or_else(|| {
        anyhow!(
            "lookup table {} is file-backed; only `source.type: processor` tables are writable",
            rule.lookup_table
        )
    })?;
    let plan: Plan<'a> = Plan::resolve(mutable.table(), &table.meta().config, rule)?;
    // One handle per rule per batch: the outcomes are tallied without atomics and
    // reach the shared counters when it drops.
    let write = mutable.write_as(node_id);

    let parent_ids = attr_input::rule_parent_ids(pl, &rule.table)?;
    let eligible = attr_input::eligible_parents(pl, shared, rule.when.as_ref(), &parent_ids)?;
    let is_eligible = |parent| eligible.as_ref().is_none_or(|set| set.contains(&parent));

    let pivot = rule
        .pivot
        .as_ref()
        .map(|pivot| PivotData::read(pl, &rule.table, pivot, &plan))
        .transpose()?;

    // Nulls are kept: an attribute that is present but explicitly null is a null
    // cell for a value column, which is not the same as the attribute being
    // absent. For a key or bound column both still skip the row.
    let source = |binding: &Binding<'a>| -> Result<ColumnSource<'a>> {
        Ok(match binding {
            Binding::Key(input) => ColumnSource::Attr(attr_input::read_source(
                pl,
                &rule.table,
                input.table.as_ref(),
                input.key.as_deref().expect("a key binding has a key"),
                &parent_ids,
                true,
            )?),
            Binding::Pivot { slot, field, .. } => ColumnSource::Pivot { slot: *slot, field },
        })
    };
    let column_sources = plan
        .columns
        .iter()
        .map(|(binding, _)| source(binding))
        .collect::<Result<Vec<_>>>()?;
    let bound_sources = plan
        .bounds
        .as_ref()
        .map(|bounds| Ok::<_, anyhow::Error>((source(&bounds.start)?, source(&bounds.end)?)))
        .transpose()?;

    // A parent carrying none of the inputs is not a candidate — it is not
    // participating, and an error for it would drown the counter. One carrying
    // *some* is a candidate, and fails when the row is built.
    let mut set = FxHashSet::default();
    for source in column_sources
        .iter()
        .chain(bound_sources.iter().flat_map(|(start, end)| [start, end]))
    {
        if let ColumnSource::Attr(source) = source {
            source.extend_candidates(&is_eligible, &mut set);
        }
    }
    if let Some(pivot) = &pivot {
        pivot.extend_candidates(&is_eligible, &mut set);
    }
    let candidates = attr_input::sorted(set);

    let mut converted = Vec::with_capacity(plan.columns.len());
    for parent in candidates {
        converted.clear();
        let mut usable = true;
        for (idx, ((binding, ty), source)) in plan.columns.iter().zip(&column_sources).enumerate() {
            match source
                .get(parent, pivot.as_ref())
                .and_then(|value| cell_of(value, binding.cast(), *ty))
            {
                // A null key is not a key: the row would be stored under a
                // placeholder no probe could ever name.
                Some(cell)
                    if cell.as_ref().is_null()
                        && plan.key_columns.contains(&(idx as ColumnIdx)) =>
                {
                    usable = false;
                }
                Some(cell) => converted.push(cell),
                None => usable = false,
            }
            if !usable {
                break;
            }
        }

        let interval = match (&plan.bounds, &bound_sources) {
            (Some(bounds), Some((start_source, end_source))) if usable => {
                let point = |binding: &Binding<'a>, source: &ColumnSource<'a>| {
                    source
                        .get(parent, pivot.as_ref())
                        .and_then(|value| range_point(value, binding.cast(), bounds.range_type))
                };
                match (
                    point(&bounds.start, start_source),
                    point(&bounds.end, end_source),
                ) {
                    (Some(start), Some(end)) => Some((start, end)),
                    _ => {
                        usable = false;
                        None
                    }
                }
            }
            _ => None,
        };

        if !usable {
            write.note_key_error();
            continue;
        }

        let cells: SmallVec<[CellRef<'_>; 8]> = converted
            .iter()
            .map(attr_input::Converted::as_ref)
            .collect();
        // The handle tallies every outcome, and neither a `Rejected` nor an
        // `Err` is worth failing the rule for.
        let outcome = match (&plan.bounds, interval) {
            (Some(bounds), Some((start, end))) => write.upsert_range(
                &cells,
                start,
                bounds.start_inclusive,
                end,
                bounds.end_inclusive,
            ),
            _ => write.upsert_hash(&cells),
        };
        if let Err(e) = outcome {
            trace!(
                error = %e,
                lookup_table = %rule.lookup_table,
                "lookup table rejected a row"
            );
        }
    }
    Ok(())
}

// ---- values ----

/// Convert an attribute value into one column cell.
///
/// Without a cast the value must already be the column's declared type — no
/// implicit stringification, and `"42"` never becomes an integer. With one
/// (already checked against `ty` by [`Plan::resolve`]) a string goes through
/// [`CellRef::parse`], the parser the table was built with, so a collected IP
/// and a probing IP agree down to the IPv4-mapped form.
fn cell_of<'a>(
    value: &'a Value<'_>,
    cast: Option<AttributeCollectCast>,
    ty: lt::ValueType,
) -> Option<attr_input::Converted<'a>> {
    let converted = attr_input::convert(value, cast.map(cast_type))?;
    let matches = matches!(
        (converted.as_ref(), ty),
        (CellRef::Null, _)
            | (CellRef::Bool(_), lt::ValueType::Bool)
            | (CellRef::Integer(_), lt::ValueType::Integer)
            | (CellRef::Double(_), lt::ValueType::Double)
            | (CellRef::String(_), lt::ValueType::String)
            | (CellRef::Ip(_), lt::ValueType::Ip)
    );
    matches.then_some(converted)
}

fn range_point(
    value: &Value<'_>,
    cast: Option<AttributeCollectCast>,
    range_type: lt::RangeType,
) -> Option<RangePoint> {
    let ty = match range_type {
        lt::RangeType::Integer => lt::ValueType::Integer,
        lt::RangeType::Ip => lt::ValueType::Ip,
    };
    RangePoint::from_cell(cell_of(value, cast, ty)?.as_ref(), range_type)
}

fn cast_type(cast: AttributeCollectCast) -> lt::ValueType {
    match cast {
        AttributeCollectCast::Bool => lt::ValueType::Bool,
        AttributeCollectCast::Double => lt::ValueType::Double,
        AttributeCollectCast::Integer => lt::ValueType::Integer,
        AttributeCollectCast::String => lt::ValueType::String,
        AttributeCollectCast::Ip => lt::ValueType::Ip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::lookup_table::cell::{CellOwned, KeyRef, Lookup};
    use crate::common::internal_metrics::LookupTableProcessor;
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value as AnyVal;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{AnyValue, KeyValue};
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource;
    use serde_json::{Value as Json, json};
    use std::net::IpAddr;
    use std::time::Duration;

    // ---- payload fixtures ----

    fn kv(key: &str, value: AnyVal) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue { value: Some(value) }),
        }
    }

    /// An attribute that is present but explicitly null.
    fn null_kv(key: &str) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue { value: None }),
        }
    }

    fn s(v: &str) -> AnyVal {
        AnyVal::StringValue(v.to_string())
    }

    fn int(v: i64) -> AnyVal {
        AnyVal::IntValue(v)
    }

    fn dbl(v: f64) -> AnyVal {
        AnyVal::DoubleValue(v)
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

    // ---- lookup table fixtures ----

    fn processor_source(columns: &[(&str, lt::ValueType)], max_rows: usize) -> lt::Source {
        lt::Source::Processor(lt::Processor {
            columns: columns
                .iter()
                .map(|(name, value_type)| lt::ColumnSpec {
                    name: name.to_string(),
                    value_type: *value_type,
                })
                .collect(),
            ttl: Duration::from_secs(60),
            max_rows,
        })
    }

    /// An empty processor-fed hash table for the processor to fill.
    fn hash_table(
        cache: &cache::Cache,
        key_columns: &[&str],
        columns: &[(&str, lt::ValueType)],
    ) -> Uuid {
        capped_hash_table(cache, key_columns, columns, 1000)
    }

    fn capped_hash_table(
        cache: &cache::Cache,
        key_columns: &[&str],
        columns: &[(&str, lt::ValueType)],
        max_rows: usize,
    ) -> Uuid {
        let config = lt::LookupTable {
            id: Uuid::now_v7(),
            name: "hash".into(),
            source: processor_source(columns, max_rows),
            key: lt::Key::Hash(lt::Hash {
                key_columns: key_columns.iter().map(|c| c.to_string()).collect(),
            }),
        };
        cache.lookup_table.insert(&config).unwrap();
        config.id
    }

    /// An empty processor-fed range table. `columns` includes the two bound
    /// columns `lo`/`hi`, which the index consumes.
    fn range_table(
        cache: &cache::Cache,
        range_type: lt::RangeType,
        partition_columns: &[&str],
        columns: &[(&str, lt::ValueType)],
    ) -> Uuid {
        let config = lt::LookupTable {
            id: Uuid::now_v7(),
            name: "range".into(),
            source: processor_source(columns, 1000),
            key: lt::Key::Range(lt::Range {
                range_type,
                partition_columns: partition_columns.iter().map(|c| c.to_string()).collect(),
                start_column: "lo".into(),
                start_inclusive: true,
                end_column: "hi".into(),
                end_inclusive: true,
            }),
        };
        cache.lookup_table.insert(&config).unwrap();
        config.id
    }

    /// A file-backed table, which collection must refuse to write to.
    fn file_table(cache: &cache::Cache) -> Uuid {
        let config: lt::LookupTable = serde_json::from_value(json!({
            "id": Uuid::now_v7(),
            "name": "immutable",
            "source": {
                "type": "file",
                "location": {"type": "file_system"},
                "paths": ["/nonexistent/hosts.csv"],
                "format": {"type": "csv", "has_headers": false, "headers": ["id", "name"]},
            },
            "key": {"type": "hash", "key_columns": ["id"]},
        }))
        .unwrap();
        cache.lookup_table.insert(&config).unwrap();
        config.id
    }

    // ---- reading the table back ----

    fn rows(cache: &cache::Cache, id: Uuid) -> usize {
        cache
            .lookup_table
            .get(&id)
            .unwrap()
            .as_mutable()
            .unwrap()
            .table()
            .len()
    }

    /// One stored row's cells, in schema order, or `None` when the probe misses.
    fn row(cache: &cache::Cache, id: Uuid, probe: Lookup<'_>) -> Option<Vec<CellOwned>> {
        let table = cache.lookup_table.get(&id).unwrap();
        let mutable = table.as_mutable().unwrap();
        let stored = mutable.table().get(probe)?;
        let width = mutable.table().schema().len() as ColumnIdx;
        Some((0..width).map(|i| stored.cell(i).to_owned_cell()).collect())
    }

    fn by_str(cache: &cache::Cache, id: Uuid, key: &str) -> Option<Vec<CellOwned>> {
        row(cache, id, Lookup::Hash(&[KeyRef::String(key)]))
    }

    fn text(v: &str) -> CellOwned {
        CellOwned::String(SmolStr::new(v))
    }

    // ---- driving the processor ----

    #[test]
    fn resource_only_collection_needs_no_record_attributes_and_preserves_payload() {
        let cache = cache::Cache::new();
        let id = hash_table(&cache, &["host"], &[("host", lt::ValueType::String)]);
        let cfg = config(
            json!({"collections":[{"lookup_table":id,"table":"record","columns":{"host":{"table":"resource","key":"host"}}}]}),
        );
        for pl in super::super::test_payloads::signals() {
            let original = pl.data.clone();
            let (out, errored) = run(pl, &cfg, &cache);
            assert!(!errored);
            assert_eq!(out.data, original);
            assert_eq!(by_str(&cache, id, "a"), Some(vec![text("a")]));
        }
    }

    fn config(value: Json) -> AttributeCollect {
        let parsed: AttributeCollect = serde_json::from_value(value).unwrap();
        parsed.validate().expect("test config must be valid");
        parsed
    }

    #[test]
    fn an_unowned_resource_cannot_overwrite_another_hosts_collected_row() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("site", lt::ValueType::String),
            ],
        );
        let cfg = config(json!({"collections":[{
            "lookup_table":id,"table":"record","columns":{
                "host":{"table":"resource","key":"host"},
                "site":{"key":"observed_site"}
            }
        }]}));
        for null_struct in [false, true] {
            let pl = super::super::test_payloads::logs_with_unowned_resource(null_struct);
            let original = pl.data.clone();
            let (out, errored) = run(pl, &cfg, &cache);
            assert!(!errored);
            assert_eq!(out.data, original);
            assert_eq!(
                by_str(&cache, id, "a"),
                Some(vec![text("a"), text("correct")])
            );
            assert_eq!(rows(&cache, id), 1);
        }
    }

    fn run(pl: Payload, cfg: &AttributeCollect, cache: &cache::Cache) -> (Payload, bool) {
        attribute_collect(pl, cfg, Uuid::now_v7(), cache)
    }

    /// Runs with a known node id so the per-processor counters can be read back.
    fn run_counted(
        pl: Payload,
        cfg: &AttributeCollect,
        cache: &cache::Cache,
        table: Uuid,
    ) -> LookupTableProcessor {
        let node = Uuid::now_v7();
        let _ = attribute_collect(pl, cfg, node, cache);
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

    /// Every attribute batch, so a test can prove the telemetry was untouched.
    fn snapshot(pl: &Payload) -> Vec<Option<arrow::array::RecordBatch>> {
        [
            ArrowPayloadType::LogAttrs,
            ArrowPayloadType::ScopeAttrs,
            ArrowPayloadType::ResourceAttrs,
            ArrowPayloadType::Logs,
        ]
        .iter()
        .map(|pt| pl.data.get(*pt).cloned())
        .collect()
    }

    // ---- tests ----

    #[test]
    fn a_scalar_rule_writes_one_row_per_record() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["session_id"],
            &[
                ("session_id", lt::ValueType::String),
                ("bandwidth", lt::ValueType::Integer),
            ],
        );
        let pl = logs(
            vec![],
            vec![
                vec![kv("sid", s("a")), kv("bw", s("10"))],
                vec![kv("sid", s("b")), kv("bw", s("20"))],
                // no inputs at all: not a candidate, so not an error either
                vec![kv("unrelated", s("x"))],
            ],
        );
        let cfg = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {
                "session_id": {"key": "sid"},
                "bandwidth": {"key": "bw", "cast": "integer"},
            },
        }]}));

        let before = snapshot(&pl);
        let m = run_counted(pl.clone(), &cfg, &cache, id);
        assert_eq!(rows(&cache, id), 2);
        assert_eq!(
            by_str(&cache, id, "a").unwrap(),
            vec![text("a"), CellOwned::Integer(10)]
        );
        assert_eq!(
            by_str(&cache, id, "b").unwrap(),
            vec![text("b"), CellOwned::Integer(20)]
        );
        assert_eq!(m.writes_inserted, 2);
        assert_eq!(m.writes_errored, 0);
        assert_eq!(snapshot(&pl), before, "telemetry passes through unchanged");
    }

    // ---- pivot ----

    /// One `table_format` SNMP walk row. `elements` are
    /// `(oid_name, value_formatted, value)` in *emission* order, which the walk
    /// does not keep aligned with any column order.
    fn snmp_row(id: &str, elements: &[(&str, &str, &str)]) -> Vec<KeyValue> {
        let mut attrs = vec![kv("snmp.id", s(id))];
        for (n, (name, formatted, raw)) in elements.iter().enumerate() {
            attrs.push(kv(&format!("snmp.data.{n}.oid_name"), s(name)));
            attrs.push(kv(&format!("snmp.data.{n}.value_formatted"), s(formatted)));
            attrs.push(kv(&format!("snmp.data.{n}.value"), s(raw)));
        }
        attrs
    }

    /// The interface table the pivot exists for.
    fn interfaces_table(cache: &cache::Cache) -> Uuid {
        hash_table(
            cache,
            &["ifIndex"],
            &[
                ("ifIndex", lt::ValueType::Integer),
                ("ifDescr", lt::ValueType::String),
                ("ifType", lt::ValueType::Integer),
            ],
        )
    }

    fn by_int(cache: &cache::Cache, id: Uuid, key: i64) -> Option<Vec<CellOwned>> {
        row(cache, id, Lookup::Hash(&[KeyRef::Integer(key)]))
    }

    fn pivot_config(id: Uuid, name_format: Json, columns: Json) -> AttributeCollect {
        let mut pivot =
            json!({"array": "snmp.data", "name": "oid_name", "value": "value_formatted"});
        if !name_format.is_null() {
            pivot["name_format"] = name_format;
        }
        config(json!({"collections": [{
            "lookup_table": id, "table": "record", "pivot": pivot, "columns": columns,
        }]}))
    }

    #[test]
    fn a_pivot_rule_binds_columns_by_object_name_not_by_element_position() {
        let cache = cache::Cache::new();
        let id = interfaces_table(&cache);
        // Row 34 arrives with its elements in a different order than row 33,
        // exactly what position-bound keys get wrong.
        let pl = logs(
            vec![],
            vec![
                snmp_row(
                    "33",
                    &[
                        ("IF-MIB::ifIndex.33", "33", "33"),
                        ("IF-MIB::ifDescr.33", "ifbeth9", "ifbeth9"),
                        ("IF-MIB::ifType.33", "ethernetCsmacd", "6"),
                        ("IF-MIB::ifAdminStatus.33", "up", "1"),
                    ],
                ),
                snmp_row(
                    "34",
                    &[
                        ("IF-MIB::ifType.34", "propVirtual", "53"),
                        ("IF-MIB::ifDescr.34", "lo", "lo"),
                        ("IF-MIB::ifIndex.34", "34", "34"),
                    ],
                ),
            ],
        );
        let cfg = pivot_config(
            id,
            json!({"type": "snmp_object"}),
            json!({
                "ifIndex": {"key": "snmp.id", "cast": "integer"},
                "ifDescr": null,
                // a different sibling field than the pivot default
                "ifType": {"value": "value", "cast": "integer"},
            }),
        );

        let before = snapshot(&pl);
        let m = run_counted(pl.clone(), &cfg, &cache, id);
        assert_eq!(rows(&cache, id), 2);
        assert_eq!(
            by_int(&cache, id, 33).unwrap(),
            vec![
                CellOwned::Integer(33),
                text("ifbeth9"),
                CellOwned::Integer(6)
            ]
        );
        assert_eq!(
            by_int(&cache, id, 34).unwrap(),
            vec![CellOwned::Integer(34), text("lo"), CellOwned::Integer(53)]
        );
        assert_eq!(m.writes_inserted, 2);
        assert_eq!(m.writes_errored, 0);
        assert_eq!(snapshot(&pl), before, "telemetry passes through unchanged");
    }

    #[test]
    fn snmp_object_strips_the_module_and_optionally_the_instance() {
        let cache = cache::Cache::new();
        let with_instance = hash_table(
            &cache,
            &["ifIndex"],
            &[
                ("ifIndex", lt::ValueType::Integer),
                ("ifDescr.33", lt::ValueType::String),
            ],
        );
        let row = || {
            logs(
                vec![],
                vec![snmp_row(
                    "33",
                    &[
                        // no module prefix at all, and a multi-arc instance
                        ("ifDescr.33", "ifbeth9", "ifbeth9"),
                        (
                            "IP-MIB::ipAdEntAddr.192.168.1.1",
                            "192.168.1.1",
                            "192.168.1.1",
                        ),
                    ],
                )],
            )
        };
        let columns = json!({"ifIndex": {"key": "snmp.id", "cast": "integer"}, "ifDescr.33": null});

        let cfg = pivot_config(
            with_instance,
            json!({"type": "snmp_object", "strip_instance": false}),
            columns.clone(),
        );
        let m = run_counted(row(), &cfg, &cache, with_instance);
        assert_eq!(
            by_int(&cache, with_instance, 33).unwrap(),
            vec![CellOwned::Integer(33), text("ifbeth9")]
        );
        assert_eq!(m.writes_errored, 0);

        // With the default the same element is named `ifDescr`, so the column
        // named `ifDescr.33` finds nothing and the row is incomplete.
        let stripped = hash_table(
            &cache,
            &["ifIndex"],
            &[
                ("ifIndex", lt::ValueType::Integer),
                ("ifDescr.33", lt::ValueType::String),
            ],
        );
        let cfg = pivot_config(stripped, json!({"type": "snmp_object"}), columns);
        let m = run_counted(row(), &cfg, &cache, stripped);
        assert_eq!(rows(&cache, stripped), 0);
        assert_eq!(m.writes_errored, 1);

        // Multi-arc instances strip down to the object as well.
        let addresses = hash_table(
            &cache,
            &["ifIndex"],
            &[
                ("ifIndex", lt::ValueType::Integer),
                ("ipAdEntAddr", lt::ValueType::String),
            ],
        );
        let cfg = pivot_config(
            addresses,
            json!({"type": "snmp_object"}),
            json!({"ifIndex": {"key": "snmp.id", "cast": "integer"}, "ipAdEntAddr": null}),
        );
        run(row(), &cfg, &cache);
        assert_eq!(
            by_int(&cache, addresses, 33).unwrap(),
            vec![CellOwned::Integer(33), text("192.168.1.1")]
        );
    }

    #[test]
    fn regex_extract_takes_one_group_and_a_verbatim_pivot_takes_the_raw_name() {
        let cache = cache::Cache::new();
        let id = interfaces_table(&cache);
        let pl = logs(
            vec![],
            vec![snmp_row(
                "33",
                &[
                    ("IF-MIB::ifDescr.33", "ifbeth9", "ifbeth9"),
                    // does not match `::(\w+)`, so it names nothing
                    ("ifType.33", "ethernetCsmacd", "6"),
                    ("IF-MIB::ifType.33", "ethernetCsmacd", "6"),
                ],
            )],
        );
        let cfg = pivot_config(
            id,
            json!({"type": "regex_extract", "pattern": "::(\\w+)"}),
            json!({
                "ifIndex": {"key": "snmp.id", "cast": "integer"},
                "ifDescr": null,
                "ifType": {"value": "value", "cast": "integer"},
            }),
        );
        let m = run_counted(pl, &cfg, &cache, id);
        assert_eq!(
            by_int(&cache, id, 33).unwrap(),
            vec![
                CellOwned::Integer(33),
                text("ifbeth9"),
                CellOwned::Integer(6)
            ]
        );
        assert_eq!(m.writes_errored, 0);

        // Group 0 is the whole match; no format is the raw name.
        let verbatim = hash_table(
            &cache,
            &["ifIndex"],
            &[
                ("ifIndex", lt::ValueType::Integer),
                ("IF-MIB::ifDescr.33", lt::ValueType::String),
                ("ifType", lt::ValueType::String),
            ],
        );
        let pl = logs(
            vec![],
            vec![snmp_row(
                "33",
                &[
                    ("IF-MIB::ifDescr.33", "ifbeth9", "ifbeth9"),
                    ("IF-MIB::ifType.33", "ethernetCsmacd", "6"),
                ],
            )],
        );
        let cfg = pivot_config(
            verbatim,
            Json::Null,
            json!({
                "ifIndex": {"key": "snmp.id", "cast": "integer"},
                "IF-MIB::ifDescr.33": null,
                "ifType": {"name": "IF-MIB::ifType.33"},
            }),
        );
        run(pl.clone(), &cfg, &cache);
        assert_eq!(
            by_int(&cache, verbatim, 33).unwrap(),
            vec![
                CellOwned::Integer(33),
                text("ifbeth9"),
                text("ethernetCsmacd")
            ]
        );
        let whole = hash_table(
            &cache,
            &["ifIndex"],
            &[
                ("ifIndex", lt::ValueType::Integer),
                ("ifDescr", lt::ValueType::String),
            ],
        );
        let cfg = pivot_config(
            whole,
            json!({"type": "regex_extract", "pattern": "if[A-Z]\\w+", "group": 0}),
            json!({"ifIndex": {"key": "snmp.id", "cast": "integer"}, "ifDescr": null}),
        );
        let (_, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert_eq!(
            by_int(&cache, whole, 33).unwrap(),
            vec![CellOwned::Integer(33), text("ifbeth9")]
        );
    }

    #[test]
    fn a_name_alias_feeds_a_differently_named_column() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["index"],
            &[
                ("index", lt::ValueType::Integer),
                ("description", lt::ValueType::String),
                ("alias", lt::ValueType::String),
            ],
        );
        let pl = logs(
            vec![],
            vec![snmp_row(
                "33",
                &[
                    ("IF-MIB::ifAlias.33", "uplink", "uplink"),
                    ("IF-MIB::ifDescr.33", "ifbeth9", "ifbeth9"),
                ],
            )],
        );
        let cfg = pivot_config(
            id,
            json!({"type": "snmp_object"}),
            json!({
                "index": {"key": "snmp.id", "cast": "integer"},
                "description": {"name": "ifDescr"},
                "alias": {"name": "ifAlias"},
            }),
        );
        run(pl, &cfg, &cache);
        assert_eq!(
            by_int(&cache, id, 33).unwrap(),
            vec![CellOwned::Integer(33), text("ifbeth9"), text("uplink")]
        );
    }

    #[test]
    fn a_missing_bound_object_skips_the_row_and_unrelated_records_are_not_candidates() {
        let cache = cache::Cache::new();
        let id = interfaces_table(&cache);
        let pl = logs(
            vec![],
            vec![
                // ifDescr is absent from this row: a candidate that fails
                snmp_row("33", &[("IF-MIB::ifType.33", "ethernetCsmacd", "6")]),
                // the id alone is an input, so this is a candidate too
                vec![kv("snmp.id", s("34"))],
                // the pivot array alone is an input as well
                snmp_row("", &[("IF-MIB::ifDescr.35", "x", "x")])
                    .into_iter()
                    .filter(|a| a.key != "snmp.id")
                    .collect(),
                // nothing this rule reads: not a candidate, not an error
                vec![kv("unrelated", s("x"))],
                // a name that is present but not a string names nothing
                vec![
                    kv("snmp.id", s("36")),
                    kv("snmp.data.0.oid_name", int(1)),
                    kv("snmp.data.0.value_formatted", s("x")),
                ],
            ],
        );
        let cfg = pivot_config(
            id,
            json!({"type": "snmp_object"}),
            json!({
                "ifIndex": {"key": "snmp.id", "cast": "integer"},
                "ifDescr": null,
                "ifType": {"value": "value", "cast": "integer"},
            }),
        );
        let m = run_counted(pl, &cfg, &cache, id);
        assert_eq!(rows(&cache, id), 0);
        assert_eq!(m.writes_inserted, 0);
        assert_eq!(m.writes_errored, 4);
    }

    #[test]
    fn duplicate_names_resolve_to_the_highest_element_and_nulls_are_stored() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["ifIndex"],
            &[
                ("ifIndex", lt::ValueType::Integer),
                ("ifDescr", lt::ValueType::String),
                ("ifAlias", lt::ValueType::String),
            ],
        );
        let mut attrs = snmp_row(
            "33",
            &[
                // both strip to `ifDescr`; the higher element wins
                ("IF-MIB::ifDescr.1", "first", "first"),
                ("IF-MIB::ifDescr.2", "second", "second"),
            ],
        );
        attrs.push(kv("snmp.data.2.oid_name", s("IF-MIB::ifAlias.33")));
        attrs.push(null_kv("snmp.data.2.value_formatted"));
        let pl = logs(vec![], vec![attrs]);
        let cfg = pivot_config(
            id,
            json!({"type": "snmp_object"}),
            json!({
                "ifIndex": {"key": "snmp.id", "cast": "integer"},
                "ifDescr": null,
                "ifAlias": null,
            }),
        );
        let m = run_counted(pl, &cfg, &cache, id);
        assert_eq!(
            by_int(&cache, id, 33).unwrap(),
            vec![CellOwned::Integer(33), text("second"), CellOwned::Null]
        );
        assert_eq!(m.writes_errored, 0);
    }

    #[test]
    fn a_pivoted_cast_that_contradicts_the_column_is_a_plan_error() {
        let cache = cache::Cache::new();
        let id = interfaces_table(&cache);
        let pl = logs(
            vec![],
            vec![snmp_row(
                "33",
                &[
                    ("IF-MIB::ifDescr.33", "ifbeth9", "ifbeth9"),
                    ("IF-MIB::ifType.33", "ethernetCsmacd", "6"),
                ],
            )],
        );
        let cfg = pivot_config(
            id,
            json!({"type": "snmp_object"}),
            json!({
                "ifIndex": {"key": "snmp.id", "cast": "integer"},
                "ifDescr": {"cast": "integer"},
                "ifType": {"value": "value", "cast": "integer"},
            }),
        );
        let before = snapshot(&pl);
        let (pl, errored) = run(pl, &cfg, &cache);
        assert!(errored);
        assert_eq!(rows(&cache, id), 0);
        assert_eq!(snapshot(&pl), before);
    }

    #[test]
    fn a_pivoted_binding_may_supply_a_range_bound() {
        let cache = cache::Cache::new();
        let id = range_table(
            &cache,
            lt::RangeType::Integer,
            &[],
            &[
                ("lo", lt::ValueType::Integer),
                ("hi", lt::ValueType::Integer),
                ("owner", lt::ValueType::String),
            ],
        );
        let pl = logs(
            vec![],
            vec![snmp_row(
                "1",
                &[
                    ("MY-MIB::rangeEnd.1", "200", "200"),
                    ("MY-MIB::rangeStart.1", "100", "100"),
                    ("MY-MIB::rangeOwner.1", "core", "core"),
                ],
            )],
        );
        let cfg = pivot_config(
            id,
            json!({"type": "snmp_object"}),
            json!({
                "lo": {"name": "rangeStart", "cast": "integer"},
                "hi": {"name": "rangeEnd", "cast": "integer"},
                "owner": {"name": "rangeOwner"},
            }),
        );
        let m = run_counted(pl, &cfg, &cache, id);
        assert_eq!(m.writes_inserted, 1);
        assert_eq!(
            row(
                &cache,
                id,
                Lookup::Range {
                    partition: &[],
                    point: RangePoint::Integer(150),
                }
            )
            .unwrap(),
            vec![text("core")]
        );
    }

    #[test]
    fn a_resource_rule_writes_one_row_for_the_shared_parent() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["host"],
            &[
                ("host", lt::ValueType::String),
                ("env", lt::ValueType::String),
            ],
        );
        let pl = logs(
            vec![kv("host.id", s("router")), kv("env", s("prod"))],
            vec![vec![kv("a", s("1"))], vec![kv("a", s("2"))]],
        );
        let cfg = config(json!({"collections": [{
            "lookup_table": id, "table": "resource",
            "columns": {"host": {"key": "host.id"}, "env": {"key": "env"}},
        }]}));

        let m = run_counted(pl, &cfg, &cache, id);
        assert_eq!(rows(&cache, id), 1, "two records, one shared resource");
        assert_eq!(
            by_str(&cache, id, "router").unwrap(),
            vec![text("router"), text("prod")]
        );
        assert_eq!(m.writes_inserted, 1);
    }

    #[test]
    fn a_repeated_key_replaces_the_whole_row_rather_than_merging_it() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["sid"],
            &[
                ("sid", lt::ValueType::String),
                ("state", lt::ValueType::String),
            ],
        );
        let cfg = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {"sid": {"key": "sid"}, "state": {"key": "state"}},
        }]}));

        let (_, errored) = run(
            logs(vec![], vec![vec![kv("sid", s("a")), kv("state", s("up"))]]),
            &cfg,
            &cache,
        );
        assert!(!errored);
        let m = run_counted(
            logs(
                vec![],
                vec![vec![kv("sid", s("a")), kv("state", s("down"))]],
            ),
            &cfg,
            &cache,
            id,
        );
        assert_eq!(rows(&cache, id), 1);
        assert_eq!(
            by_str(&cache, id, "a").unwrap(),
            vec![text("a"), text("down")]
        );
        assert_eq!(m.writes_updated, 1);
        assert_eq!(m.writes_inserted, 0);
    }

    #[test]
    fn an_explicit_null_is_stored_in_a_value_column_and_skips_a_key_column() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["sid"],
            &[
                ("sid", lt::ValueType::String),
                ("state", lt::ValueType::String),
            ],
        );
        let pl = logs(
            vec![],
            vec![
                // present but null value column -> stored as null
                vec![kv("sid", s("a")), null_kv("state")],
                // null key -> no key to store the row under
                vec![null_kv("sid"), kv("state", s("up"))],
                // the value attribute is absent entirely -> incomplete row
                vec![kv("sid", s("c"))],
            ],
        );
        let cfg = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {"sid": {"key": "sid"}, "state": {"key": "state"}},
        }]}));

        let m = run_counted(pl, &cfg, &cache, id);
        assert_eq!(rows(&cache, id), 1);
        assert_eq!(
            by_str(&cache, id, "a").unwrap(),
            vec![text("a"), CellOwned::Null]
        );
        assert_eq!(m.writes_inserted, 1);
        assert_eq!(m.writes_errored, 2, "a null key and a missing input");
    }

    #[test]
    fn casts_reuse_the_parser_the_table_was_built_with() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["ip"],
            &[
                ("ip", lt::ValueType::Ip),
                ("up", lt::ValueType::Bool),
                ("load", lt::ValueType::Double),
                ("port", lt::ValueType::Integer),
            ],
        );
        let pl = logs(
            vec![],
            vec![vec![
                // the v4-mapped form folds onto v4, as it does on the probe side
                kv("ip", s("::ffff:10.0.0.1")),
                kv("up", s("yes")),
                kv("load", s("0.5")),
                kv("port", int(8080)),
            ]],
        );
        let cfg = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {
                "ip": {"key": "ip", "cast": "ip"},
                "up": {"key": "up", "cast": "bool"},
                "load": {"key": "load", "cast": "double"},
                "port": {"key": "port", "cast": "integer"},
            },
        }]}));

        let m = run_counted(pl, &cfg, &cache, id);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert_eq!(
            row(&cache, id, Lookup::Hash(&[KeyRef::Ip(ip)])).unwrap(),
            vec![
                CellOwned::Ip(ip),
                CellOwned::Bool(true),
                CellOwned::Double(0.5),
                CellOwned::Integer(8080),
            ]
        );
        assert_eq!(m.writes_inserted, 1);
    }

    /// An `ip` column without `cast: ip` used to plan fine and then drop every
    /// candidate carrying a real address while storing the explicitly null
    /// ones, so the table filled with null-IP rows and looked alive.
    #[test]
    fn an_ip_column_without_a_cast_is_a_plan_error_not_a_silent_row_drop() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["sid"],
            &[("sid", lt::ValueType::String), ("ip", lt::ValueType::Ip)],
        );
        let payload = || {
            logs(
                vec![],
                vec![
                    vec![kv("sid", s("a")), kv("ip", s("10.0.0.1"))],
                    // the row that used to be stored while the real one was dropped
                    vec![kv("sid", s("b")), null_kv("ip")],
                ],
            )
        };

        let uncast = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {"sid": {"key": "sid"}, "ip": {"key": "ip"}},
        }]}));
        let (_, errored) = run(payload(), &uncast, &cache);
        assert!(errored, "an uncast ip column must fail the rule");
        assert_eq!(rows(&cache, id), 0, "no row may be written on a plan error");

        // and the same rule with the cast spelled out stores both rows
        let cast = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {"sid": {"key": "sid"}, "ip": {"key": "ip", "cast": "ip"}},
        }]}));
        let (_, errored) = run(payload(), &cast, &cache);
        assert!(!errored);
        assert_eq!(
            by_str(&cache, id, "a"),
            Some(vec![text("a"), CellOwned::Ip("10.0.0.1".parse().unwrap())])
        );
        assert_eq!(
            by_str(&cache, id, "b"),
            Some(vec![text("b"), CellOwned::Null])
        );
    }

    #[test]
    fn numbers_are_not_reinterpreted_across_types() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["sid"],
            &[
                ("sid", lt::ValueType::String),
                ("load", lt::ValueType::Double),
            ],
        );
        let cfg = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {"sid": {"key": "sid"}, "load": {"key": "load", "cast": "double"}},
        }]}));

        // An int attribute into a double column is a failed conversion, not a
        // widening; the whole row is skipped.
        let m = run_counted(
            logs(vec![], vec![vec![kv("sid", s("a")), kv("load", int(1))]]),
            &cfg,
            &cache,
            id,
        );
        assert_eq!(rows(&cache, id), 0);
        assert_eq!(m.writes_errored, 1);

        // Uncast, the attribute has to arrive as the column's type already: a
        // string is not silently parsed into the double column.
        let uncast = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {"sid": {"key": "sid"}, "load": {"key": "load"}},
        }]}));
        let (_, errored) = run(
            logs(vec![], vec![vec![kv("sid", s("a")), kv("load", s("1.5"))]]),
            &uncast,
            &cache,
        );
        assert!(!errored, "a skipped row is not a rule failure");
        assert_eq!(rows(&cache, id), 0);

        let (_, errored) = run(
            logs(vec![], vec![vec![kv("sid", s("a")), kv("load", dbl(1.5))]]),
            &uncast,
            &cache,
        );
        assert!(!errored);
        assert_eq!(
            by_str(&cache, id, "a").unwrap(),
            vec![text("a"), CellOwned::Double(1.5)]
        );
    }

    #[test]
    fn a_range_rule_takes_its_bounds_from_the_bound_columns() {
        let cache = cache::Cache::new();
        let id = range_table(
            &cache,
            lt::RangeType::Integer,
            &["vrf"],
            &[
                ("vrf", lt::ValueType::String),
                ("lo", lt::ValueType::Integer),
                ("hi", lt::ValueType::Integer),
                ("zone", lt::ValueType::String),
            ],
        );
        let pl = logs(
            vec![],
            vec![
                vec![
                    kv("vrf", s("blue")),
                    kv("lo", int(10)),
                    kv("hi", int(20)),
                    kv("zone", s("dmz")),
                ],
                // a null bound has no place in an index
                vec![
                    kv("vrf", s("blue")),
                    null_kv("lo"),
                    kv("hi", int(40)),
                    kv("zone", s("core")),
                ],
            ],
        );
        let cfg = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {
                "vrf": {"key": "vrf"},
                "lo": {"key": "lo", "cast": "integer"},
                "hi": {"key": "hi", "cast": "integer"},
                "zone": {"key": "zone"},
            },
        }]}));

        let m = run_counted(pl, &cfg, &cache, id);
        let probe = |point: i64| {
            row(
                &cache,
                id,
                Lookup::Range {
                    partition: &[KeyRef::String("blue")],
                    point: RangePoint::Integer(point),
                },
            )
        };
        // The table's own `end_inclusive: true` decides the closed upper end,
        // and the bounds are consumed by the index rather than stored.
        assert_eq!(probe(20).unwrap(), vec![text("blue"), text("dmz")]);
        assert_eq!(probe(9), None);
        assert_eq!(probe(21), None);
        assert_eq!(m.writes_inserted, 1);
        assert_eq!(m.writes_errored, 1, "the null lower bound");
    }

    #[test]
    fn an_ip_range_needs_an_explicit_cast_and_canonicalises_its_bounds() {
        let cache = cache::Cache::new();
        let columns = &[
            ("lo", lt::ValueType::Ip),
            ("hi", lt::ValueType::Ip),
            ("city", lt::ValueType::String),
        ];
        let id = range_table(&cache, lt::RangeType::Ip, &[], columns);
        let pl = || {
            logs(
                vec![],
                vec![vec![
                    kv("lo", s("::ffff:10.0.0.0")),
                    kv("hi", s("10.0.0.255")),
                    kv("city", s("jakarta")),
                ]],
            )
        };

        // OTAP has no IP value, so without the cast the rule cannot run at all.
        let uncast = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {"lo": {"key": "lo"}, "hi": {"key": "hi"}, "city": {"key": "city"}},
        }]}));
        let (_, errored) = run(pl(), &uncast, &cache);
        assert!(errored, "an ip range must demand `cast: ip`");
        assert_eq!(rows(&cache, id), 0);

        let cfg = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {
                "lo": {"key": "lo", "cast": "ip"},
                "hi": {"key": "hi", "cast": "ip"},
                "city": {"key": "city"},
            },
        }]}));
        let (_, errored) = run(pl(), &cfg, &cache);
        assert!(!errored);
        assert_eq!(
            row(
                &cache,
                id,
                Lookup::Range {
                    partition: &[],
                    point: RangePoint::Ip("10.0.0.7".parse().unwrap()),
                },
            )
            .unwrap(),
            vec![text("jakarta")],
            "the v4-mapped lower bound folds onto v4"
        );
    }

    #[test]
    fn guards_narrow_which_records_are_collected() {
        let cache = cache::Cache::new();
        let id = hash_table(&cache, &["sid"], &[("sid", lt::ValueType::String)]);
        let pl = logs(
            vec![kv("env", s("prod"))],
            vec![
                vec![kv("sid", s("a")), kv("event", s("inventory"))],
                vec![kv("sid", s("b")), kv("event", s("other"))],
            ],
        );
        let cfg = config(json!({
            "when": predicate("resource", "env", "prod"),
            "collections": [{
                "lookup_table": id, "table": "record",
                "when": predicate("record", "event", "inventory"),
                "columns": {"sid": {"key": "sid"}},
            }],
        }));

        let (_, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert_eq!(rows(&cache, id), 1);
        assert!(by_str(&cache, id, "a").is_some());
        assert!(by_str(&cache, id, "b").is_none());

        // The shared guard alone can exclude everything.
        let staging = logs(
            vec![kv("env", s("staging"))],
            vec![vec![kv("sid", s("c")), kv("event", s("inventory"))]],
        );
        let (_, errored) = run(staging, &cfg, &cache);
        assert!(!errored);
        assert_eq!(rows(&cache, id), 1);
    }

    #[test]
    fn capacity_rejections_are_counted_apart_from_errors() {
        let cache = cache::Cache::new();
        let id = capped_hash_table(&cache, &["sid"], &[("sid", lt::ValueType::String)], 1);
        let pl = logs(
            vec![],
            vec![vec![kv("sid", s("a"))], vec![kv("sid", s("b"))]],
        );
        let cfg = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {"sid": {"key": "sid"}},
        }]}));

        let m = run_counted(pl, &cfg, &cache, id);
        assert_eq!(rows(&cache, id), 1);
        assert_eq!(m.writes_inserted, 1);
        assert_eq!(m.writes_rejected, 1);
        assert_eq!(m.writes_errored, 0);
    }

    #[test]
    fn a_misconfigured_rule_is_skipped_without_dropping_records() {
        let cache = cache::Cache::new();
        let hash = hash_table(
            &cache,
            &["sid"],
            &[
                ("sid", lt::ValueType::String),
                ("bw", lt::ValueType::Integer),
            ],
        );
        let double_keyed = hash_table(&cache, &["load"], &[("load", lt::ValueType::Double)]);
        let file = file_table(&cache);
        let pl = || logs(vec![], vec![vec![kv("sid", s("a")), kv("bw", int(1))]]);
        let rule = |value: Json| {
            let cfg = config(json!({"collections": [value]}));
            let before = snapshot(&pl());
            let (out, errored) = run(pl(), &cfg, &cache);
            assert_eq!(snapshot(&out), before, "telemetry survives a bad rule");
            errored
        };

        assert!(
            rule(json!({
                "lookup_table": hash, "table": "record",
                "columns": {"sid": {"key": "sid"}},
            })),
            "every declared column must be bound"
        );
        assert!(
            rule(json!({
                "lookup_table": hash, "table": "record",
                "columns": {
                    "sid": {"key": "sid"}, "bw": {"key": "bw", "cast": "integer"},
                    "nope": {"key": "x"},
                },
            })),
            "an unknown column is rejected"
        );
        assert!(
            rule(json!({
                "lookup_table": hash, "table": "record",
                "columns": {"sid": {"key": "sid"}, "bw": {"key": "bw", "cast": "string"}},
            })),
            "a cast has to name the column's own type"
        );
        assert!(
            rule(json!({
                "lookup_table": double_keyed, "table": "record",
                "columns": {"load": {"key": "bw", "cast": "double"}},
            })),
            "a double cannot index a table"
        );
        assert!(
            rule(json!({
                "lookup_table": file, "table": "record",
                "columns": {"id": {"key": "sid"}, "name": {"key": "sid"}},
            })),
            "a file-backed table is not writable"
        );
        assert!(
            rule(json!({
                "lookup_table": Uuid::now_v7(), "table": "record",
                "columns": {"sid": {"key": "sid"}},
            })),
            "an unconfigured table is a misconfiguration"
        );
        assert_eq!(rows(&cache, hash), 0);
        assert_eq!(rows(&cache, double_keyed), 0);
    }

    #[test]
    fn independent_rules_may_target_the_same_table() {
        let cache = cache::Cache::new();
        let id = hash_table(
            &cache,
            &["sid"],
            &[
                ("sid", lt::ValueType::String),
                ("state", lt::ValueType::String),
            ],
        );
        let pl = logs(
            vec![],
            vec![vec![
                kv("primary", s("a")),
                kv("secondary", s("b")),
                kv("state", s("up")),
            ]],
        );
        let cfg = config(json!({"collections": [
            {
                "lookup_table": id, "table": "record",
                "columns": {"sid": {"key": "primary"}, "state": {"key": "state"}},
            },
            {
                "lookup_table": id, "table": "record",
                "columns": {"sid": {"key": "secondary"}, "state": {"key": "state"}},
            },
        ]}));

        let (_, errored) = run(pl, &cfg, &cache);
        assert!(!errored);
        assert_eq!(rows(&cache, id), 2);
        assert!(by_str(&cache, id, "a").is_some());
        assert!(by_str(&cache, id, "b").is_some());
    }

    #[test]
    fn an_empty_payload_is_returned_untouched() {
        let cache = cache::Cache::new();
        let id = hash_table(&cache, &["sid"], &[("sid", lt::ValueType::String)]);
        let cfg = config(json!({"collections": [{
            "lookup_table": id, "table": "record",
            "columns": {"sid": {"key": "sid"}},
        }]}));

        let (_, errored) = run(logs(vec![], vec![]), &cfg, &cache);
        assert!(!errored);
        assert_eq!(rows(&cache, id), 0);
    }
}
