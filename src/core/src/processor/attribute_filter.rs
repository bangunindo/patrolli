// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::config::processor::AttributeFilter;
use crate::payload::{self, attr};
use anyhow::Result;
use otel_arrow_dfe_pdata::otap::filter::{IdBitmapPool, filter_otap_batch};

/// Drop every record that does not match `filter`, keeping the rest. The matching
/// per-record mask is evaluated by [`attr::attribute_filter_mask`] (shared with the
/// listener's `AttributeMatch` path); `filter_otap_batch` then keeps the `true` rows
/// and cascades the selection down to the child attribute tables by id.
pub(super) fn attribute_filter(
    pl: payload::Payload,
    filter: &AttributeFilter,
) -> Result<payload::Payload> {
    if pl.is_empty() {
        return Ok(pl);
    }
    let mask = attr::attribute_filter_mask(&pl, filter)?;
    let mut pool = IdBitmapPool::new();
    let filtered = filter_otap_batch(&mask, &pl.data, &mut pool)
        .map_err(|e| anyhow::anyhow!("attribute_filter: {e}"))?;
    Ok(payload::Payload::new(filtered))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::processor::{AttributeFilterExpr, AttributeTable, KeyValueOp, Value};
    use crate::processor::test_payloads::{attr_strs, rows, tagged_signals};
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType::{self, *};

    fn matches(
        table: AttributeTable,
        key: &str,
        key_is_regex: bool,
        value: &str,
    ) -> AttributeFilter {
        AttributeFilter::Match(AttributeFilterExpr {
            table,
            key: key.into(),
            key_is_regex,
            op: KeyValueOp::Equal,
            value: Value::String(value.into()),
        })
    }

    /// The table of record attributes, and every table under a record.
    const TABLES: [(ArrowPayloadType, &[ArrowPayloadType]); 3] = [
        (LogAttrs, &[]),
        (MetricAttrs, &[NumberDataPoints, NumberDpAttrs]),
        (
            SpanAttrs,
            &[SpanEvents, SpanEventAttrs, SpanLinks, SpanLinkAttrs],
        ),
    ];

    /// Dropping a record drops what hangs off it: its attributes, a metric's
    /// data points and theirs, a span's events and links and theirs. What the
    /// records share, the resource and the scope, stays.
    #[test]
    fn filter_cascades_to_child_tables_per_signal() {
        let filter = matches(AttributeTable::Record, "keep", false, "yes");
        for (pl, (attrs, children)) in tagged_signals().into_iter().zip(TABLES) {
            let before: Vec<usize> = children.iter().map(|&t| rows(&pl, t)).collect();
            let items = pl.num_items();
            let out = attribute_filter(pl, &filter).unwrap();

            assert_eq!(out.num_items(), items / 2, "{attrs:?}");
            assert_eq!(attr_strs(&out, attrs, "keep"), ["yes"], "{attrs:?}");
            for (&table, before) in children.iter().zip(before) {
                assert_eq!(rows(&out, table), before / 2, "{table:?}");
            }
            assert_eq!(attr_strs(&out, ResourceAttrs, "host"), ["a"], "{attrs:?}");
            assert_eq!(attr_strs(&out, ScopeAttrs, "lib"), ["x"], "{attrs:?}");
        }
    }

    #[test]
    fn a_filter_on_the_scope_or_resource_keeps_or_drops_every_record_under_it() {
        for pl in tagged_signals() {
            let items = pl.num_items();
            let scope = matches(AttributeTable::Scope, "lib", false, "x");
            assert_eq!(
                attribute_filter(pl.clone(), &scope).unwrap().num_items(),
                items
            );
            let other = matches(AttributeTable::Resource, "host", false, "b");
            assert_eq!(attribute_filter(pl, &other).unwrap().num_items(), 0);
        }
    }

    #[test]
    fn a_key_is_regex_filter_matches_the_key_by_pattern() {
        let filter = matches(AttributeTable::Record, "^ke+p$", true, "no");
        for (pl, (attrs, _)) in tagged_signals().into_iter().zip(TABLES) {
            let items = pl.num_items();
            let out = attribute_filter(pl, &filter).unwrap();
            assert_eq!(out.num_items(), items / 2, "{attrs:?}");
            assert_eq!(attr_strs(&out, attrs, "keep"), ["no"], "{attrs:?}");
        }
    }
}
