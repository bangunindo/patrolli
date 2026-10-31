// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::config::processor::TableTransform;
use crate::payload::{self};
use anyhow::Result;
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::otap::transform::apply_attribute_transform;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;

pub(super) fn attribute_transform(
    mut pl: payload::Payload,
    transforms: &TableTransform,
) -> Result<payload::Payload> {
    if let Some(transform) = &transforms.record {
        let payload_type = match &pl.data {
            OtapArrowRecords::Logs(_) => ArrowPayloadType::LogAttrs,
            OtapArrowRecords::Metrics(_) => ArrowPayloadType::MetricAttrs,
            OtapArrowRecords::Traces(_) => ArrowPayloadType::SpanAttrs,
        };

        let compute_stats_opt =
            apply_attribute_transform(&mut pl.data, payload_type, transform, false)?;
        _ = compute_stats_opt;
    }

    if let Some(transform) = &transforms.scope {
        let payload_type = ArrowPayloadType::ScopeAttrs;

        let compute_stats_opt =
            apply_attribute_transform(&mut pl.data, payload_type, transform, false)?;
        _ = compute_stats_opt;
    }

    if let Some(transform) = &transforms.resource {
        let payload_type = ArrowPayloadType::ResourceAttrs;

        let compute_stats_opt =
            apply_attribute_transform(&mut pl.data, payload_type, transform, false)?;
        _ = compute_stats_opt;
    }

    Ok(pl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::processor::{AddAttributes, Config, DropAttributes, RenameAttributes};
    use crate::processor::test_payloads::{attr_strs, tagged_signals};
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType::*;

    fn transforms(r#type: &str, config: serde_json::Value) -> TableTransform {
        let mut processor: Config =
            serde_json::from_value(serde_json::json!({"type": r#type, "config": config})).unwrap();
        processor.validate().unwrap();
        match processor {
            Config::DropAttributes(DropAttributes { transforms, .. })
            | Config::AddAttributes(AddAttributes { transforms, .. })
            | Config::RenameAttributes(RenameAttributes { transforms, .. }) => transforms.unwrap(),
            _ => unreachable!(),
        }
    }

    /// For each signal, the record attribute table; then, per table name the
    /// config uses, the table it is, the key the fixture puts there and its
    /// values.
    fn tables(
        pl: &payload::Payload,
    ) -> [(
        &'static str,
        ArrowPayloadType,
        &'static str,
        Vec<&'static str>,
    ); 3] {
        let record = match &pl.data {
            OtapArrowRecords::Logs(_) => LogAttrs,
            OtapArrowRecords::Metrics(_) => MetricAttrs,
            OtapArrowRecords::Traces(_) => SpanAttrs,
        };
        [
            ("record", record, "keep", vec!["yes", "no"]),
            ("scope", ScopeAttrs, "lib", vec!["x"]),
            ("resource", ResourceAttrs, "host", vec!["a"]),
        ]
    }

    /// Each of drop, add and rename, aimed at each of the three tables, on
    /// each signal: it changes that table and leaves the other two alone.
    #[test]
    fn drop_add_rename_apply_at_record_scope_resource_for_each_signal() {
        for pl in tagged_signals() {
            let tables = tables(&pl);
            for (name, table, key, values) in &tables {
                let untouched = || tables.iter().filter(|(other, ..)| other != name);
                let check_untouched = |out: &payload::Payload| {
                    for (_, table, key, values) in untouched() {
                        assert_eq!(attr_strs(out, *table, key), *values, "{name} {table:?}");
                    }
                };

                let drop = transforms(
                    "drop_attributes",
                    serde_json::json!({"keys": {*name: [key]}}),
                );
                let out = attribute_transform(pl.clone(), &drop).unwrap();
                assert!(
                    attr_strs(&out, *table, key).is_empty(),
                    "drop {name} {table:?}"
                );
                check_untouched(&out);

                let add = transforms(
                    "add_attributes",
                    serde_json::json!({
                        "kv_map": {*name: {"added": {"type": "str", "value": "v"}}},
                        "overwrite": false,
                    }),
                );
                let out = attribute_transform(pl.clone(), &add).unwrap();
                assert_eq!(
                    attr_strs(&out, *table, "added"),
                    vec!["v"; values.len()],
                    "add {name} {table:?}"
                );
                assert_eq!(
                    attr_strs(&out, *table, key),
                    *values,
                    "add {name} {table:?}"
                );
                check_untouched(&out);

                let rename = transforms(
                    "rename_attributes",
                    serde_json::json!({"rename_map": {*name: {*key: "renamed"}}}),
                );
                let out = attribute_transform(pl.clone(), &rename).unwrap();
                assert!(
                    attr_strs(&out, *table, key).is_empty(),
                    "rename {name} {table:?}"
                );
                assert_eq!(
                    attr_strs(&out, *table, "renamed"),
                    *values,
                    "rename {name} {table:?}"
                );
                check_untouched(&out);
            }
        }
    }

    #[test]
    fn add_attributes_overwrite_false_keeps_existing_true_replaces() {
        let add = |overwrite| {
            transforms(
                "add_attributes",
                serde_json::json!({
                    "kv_map": {"record": {"keep": {"type": "str", "value": "z"}}},
                    "overwrite": overwrite,
                }),
            )
        };
        let logs = tagged_signals().remove(0);
        let kept = attribute_transform(logs.clone(), &add(false)).unwrap();
        assert_eq!(attr_strs(&kept, LogAttrs, "keep"), ["yes", "no"]);
        let replaced = attribute_transform(logs, &add(true)).unwrap();
        assert_eq!(attr_strs(&replaced, LogAttrs, "keep"), ["z", "z"]);
    }

    /// A metric's record attributes are the metric's own, not its data
    /// points': a record-level drop leaves the data points' attributes.
    #[test]
    fn a_record_transform_on_metrics_leaves_data_point_attributes() {
        let metrics = tagged_signals().remove(1);
        let drop = transforms(
            "drop_attributes",
            serde_json::json!({"keys": {"record": ["dp"]}}),
        );
        let out = attribute_transform(metrics, &drop).unwrap();
        assert_eq!(attr_strs(&out, NumberDpAttrs, "dp").len(), 4);
    }
}
