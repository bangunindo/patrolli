// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The read half shared by the attribute lookup and collect processors.
//!
//! Both read the *incoming* attributes of one payload, keyed by parent id, and
//! narrow those with the same two guards; only what they do with a parent
//! differs. Defining it once keeps the two from drifting on the parts that are
//! easy to get subtly wrong: the record-reads-resource indirection and parent
//! selection.

use crate::cache::lookup_table::cell::CellRef;
use crate::config::lookup_table::ValueType;
use crate::config::processor::{AttributeFilter, AttributeTable};
use crate::payload::attr::AttrLocation;
use crate::payload::{Payload, Value, attr};
use anyhow::Result;
use arrow::array::{Array, BooleanArray, RecordBatch};
use arrow::compute::kernels;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use rustc_hash::{FxHashMap, FxHashSet};
use smol_str::{SmolStr, format_smolstr};

/// A missing record id is a logical row identity, not a dangling OTAP parent.
/// It is encoded only if enrichment actually writes an attribute to that row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Parent {
    Existing(u16),
    MissingRecord(usize),
}

impl Parent {
    pub(super) fn encoded(self) -> Option<u16> {
        match self {
            Self::Existing(id) => Some(id),
            Self::MissingRecord(_) => None,
        }
    }
}

pub(super) enum Converted<'a> {
    Borrowed(CellRef<'a>),
    Text(SmolStr),
}

impl Converted<'_> {
    pub(super) fn as_ref(&self) -> CellRef<'_> {
        match self {
            Self::Borrowed(cell) => *cell,
            Self::Text(text) => CellRef::String(text.as_str()),
        }
    }
}

pub(super) fn convert<'a>(value: &'a Value<'_>, cast: Option<ValueType>) -> Option<Converted<'a>> {
    let cell = match value {
        Value::Null => CellRef::Null,
        Value::Bool(v) => CellRef::Bool(*v),
        Value::Int64(v) => CellRef::Integer(*v),
        Value::Double(v) => CellRef::Double(*v),
        Value::String(v) => CellRef::String(v.as_str()),
        _ => return None,
    };
    let Some(cast) = cast else {
        return Some(Converted::Borrowed(cell));
    };
    if let CellRef::String(text) = cell {
        return CellRef::parse(text, cast).ok().map(Converted::Borrowed);
    }
    match (cell, cast) {
        (CellRef::Null, _)
        | (CellRef::Bool(_), ValueType::Bool)
        | (CellRef::Integer(_), ValueType::Integer)
        | (CellRef::Double(_), ValueType::Double) => Some(Converted::Borrowed(cell)),
        (CellRef::Bool(v), ValueType::String) => Some(Converted::Text(format_smolstr!("{v}"))),
        (CellRef::Integer(v), ValueType::String) => Some(Converted::Text(format_smolstr!("{v}"))),
        (CellRef::Double(v), ValueType::String) => Some(Converted::Text(format_smolstr!("{v}"))),
        _ => None,
    }
}

/// One binding's inputs: one value per parent.
pub(super) enum Source<'a> {
    Scalar(FxHashMap<u16, Value<'a>>),
    /// A record rule reading a resource attribute. Kept as an indirection rather
    /// than a copy per record: many records share one resource, and
    /// `payload::Value` borrows from the batch.
    ViaResource {
        of_record: FxHashMap<Parent, u16>,
        by_resource: FxHashMap<u16, Value<'a>>,
    },
}

impl<'a> Source<'a> {
    pub(super) fn get(&self, parent: Parent) -> Option<&Value<'a>> {
        match self {
            Source::Scalar(m) => m.get(&parent.encoded()?),
            Source::ViaResource {
                of_record,
                by_resource,
            } => by_resource.get(of_record.get(&parent)?),
        }
    }

    /// Add the parents this source can supply a value for. Composite inputs
    /// align by parent, never by physical Arrow row position.
    pub(super) fn extend_candidates(
        &self,
        is_eligible: &impl Fn(Parent) -> bool,
        out: &mut FxHashSet<Parent>,
    ) {
        match self {
            Source::Scalar(m) => out.extend(
                m.keys()
                    .filter(|parent| is_eligible(Parent::Existing(**parent)))
                    .map(|parent| Parent::Existing(*parent)),
            ),
            Source::ViaResource {
                of_record,
                by_resource,
            } => out.extend(
                of_record
                    .iter()
                    .filter(|(record, resource)| {
                        by_resource.contains_key(resource) && is_eligible(**record)
                    })
                    .map(|(record, _)| *record),
            ),
        }
    }
}

/// The parent ids of a rule's own attribute table, one per root row.
///
/// ID-less records still participate in resource broadcasts and collection.
pub(super) fn rule_parent_ids(pl: &Payload, table: &AttributeTable) -> Result<Vec<Option<Parent>>> {
    let location = location_of(table);
    let parent_ids = attr::location_parent_ids(pl, location)?;
    Ok(parent_ids
        .into_iter()
        .enumerate()
        .map(|(row, id)| {
            id.map(Parent::Existing).or_else(|| {
                (location == AttrLocation::Record).then_some(Parent::MissingRecord(row))
            })
        })
        .collect())
}

/// Read one binding's attribute values out of the payload. `input_table`
/// overrides the rule's own table; `keep_nulls` decides whether a present but
/// explicitly null attribute reads as [`Value::Null`] or as absent.
pub(super) fn read_source<'p>(
    pl: &'p Payload,
    rule_table: &AttributeTable,
    input_table: Option<&AttributeTable>,
    key: &str,
    rule_ids: &[Option<Parent>],
    keep_nulls: bool,
) -> Result<Source<'p>> {
    let source_table = input_table.unwrap_or(rule_table);
    let Some(batch) = attr_batch(pl, source_table) else {
        // Nothing to read, so this binding produces no candidate.
        return Ok(Source::Scalar(FxHashMap::default()));
    };

    if source_table == rule_table {
        return Ok(Source::Scalar(attr::parent_id_typed_values(
            batch, key, keep_nulls,
        )?));
    }

    // The only cross-table read the config allows: a record rule reading a
    // resource attribute, resolved through each record's resource id.
    let by_resource = attr::parent_id_typed_values(batch, key, keep_nulls)?;
    let resource_ids = attr::location_parent_ids(pl, AttrLocation::Resource)?;
    let mut of_record = FxHashMap::default();
    for (record_id, resource_id) in rule_ids.iter().zip(&resource_ids) {
        if let (Some(record_id), Some(resource_id)) = (record_id, resource_id) {
            of_record.insert(*record_id, *resource_id);
        }
    }
    Ok(Source::ViaResource {
        of_record,
        by_resource,
    })
}

/// The parents this rule may act on, or `None` when neither guard applies and
/// every parent is eligible.
pub(super) fn eligible_parents(
    pl: &Payload,
    shared: Option<&BooleanArray>,
    own: Option<&AttributeFilter>,
    parent_ids: &[Option<Parent>],
) -> Result<Option<FxHashSet<Parent>>> {
    let own = own
        .map(|filter| attr::attribute_filter_mask(pl, filter))
        .transpose()?;
    let mask = match (shared, &own) {
        (None, None) => return Ok(None),
        (Some(mask), None) => mask.clone(),
        (None, Some(mask)) => mask.clone(),
        (Some(a), Some(b)) => kernels::boolean::and(a, b)?,
    };
    // Selection is per record; a resource or scope parent becomes eligible when
    // any of its records is, which is safe because the config forces those
    // predicates to read only their own table.
    Ok(Some(
        (0..parent_ids.len())
            .filter(|&i| mask.is_valid(i) && mask.value(i))
            .filter_map(|i| parent_ids[i])
            .collect(),
    ))
}

/// The parents worth acting on.
///
/// A parent carrying none of the inputs is not a candidate — it is not
/// participating, and an error for it would drown the counter. One carrying
/// *some* is a candidate, and fails when the key or the row is built.
pub(super) fn candidates<'s, 'p: 's>(
    sources: impl Iterator<Item = &'s Source<'p>>,
    is_eligible: impl Fn(Parent) -> bool,
) -> Vec<Parent> {
    let mut set: FxHashSet<Parent> = FxHashSet::default();
    for source in sources {
        source.extend_candidates(&is_eligible, &mut set);
    }
    sorted(set)
}

/// Deterministic iteration order for a candidate set.
pub(super) fn sorted(set: FxHashSet<Parent>) -> Vec<Parent> {
    let mut out: Vec<_> = set.into_iter().collect();
    out.sort_unstable();
    out
}

pub(super) fn location_of(table: &AttributeTable) -> AttrLocation {
    match table {
        AttributeTable::Record => AttrLocation::Record,
        AttributeTable::Scope => AttrLocation::Scope,
        AttributeTable::Resource => AttrLocation::Resource,
    }
}

pub(super) fn attr_batch<'p>(pl: &'p Payload, table: &AttributeTable) -> Option<&'p RecordBatch> {
    match table {
        AttributeTable::Record => attr::get_record_attr(pl),
        AttributeTable::Scope => pl.data.get(ArrowPayloadType::ScopeAttrs),
        AttributeTable::Resource => pl.data.get(ArrowPayloadType::ResourceAttrs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn string_conversions_borrow_the_source_buffer() {
        let text = "a long string that should never be copied just to probe a table".to_string();
        let value = Value::String(crate::payload::ZString::from_ref(&text));
        for cast in [None, Some(ValueType::String)] {
            let (converted, counts) =
                super::super::test_alloc::measure(|| convert(&value, cast).unwrap());
            assert_eq!(counts.calls, 0);
            let CellRef::String(result) = converted.as_ref() else {
                panic!("expected string")
            };
            assert_eq!(result.as_ptr(), text.as_ptr());
        }
        assert!(convert(&Value::Bool(true), Some(ValueType::Integer)).is_none());
        assert!(matches!(
            convert(&Value::Int64(42), Some(ValueType::String)),
            Some(Converted::Text(_))
        ));
    }
}
