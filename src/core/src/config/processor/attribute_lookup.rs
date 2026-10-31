// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Configuration for batched attribute enrichment; `processor::attribute_lookup`
//! executes it. Validation here is table-independent: a rule is resolved against
//! the table's schema per batch, because a CSV table's columns are known only
//! once it has loaded and can change on reload.

use super::{AttributeFilter, AttributeTable};
use anyhow::{Context, Result, bail};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;
use uuid::Uuid;

/// Enrich attributes with independent lookups, applying their writes together.
///
/// Every rule reads the incoming attributes: another rule's outputs are not
/// visible as inputs. Each parent matches at most one lookup row, whose
/// selected columns supply scalar output attributes.
///
/// Missing inputs, failed casts, lookup misses and null result cells leave the
/// destination unchanged; invalid inputs are counted separately from misses.
/// Defaults and fallback expressions are not supported.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributeLookup {
    /// Record-selection predicate over incoming attributes, evaluated once for
    /// all rules and ANDed with each rule's own `when`. Omitted means every
    /// record is eligible; unselected records pass through unchanged. A
    /// predicate error skips all enrichment without dropping telemetry.
    ///
    /// For a resource or scope destination every predicate leaf must reference
    /// that same table, so eligibility is uniform for its shared parent; a
    /// record destination may reference any table. Selection is per record and
    /// does not change output-collision rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<AttributeFilter>,
    /// Replace existing destination values. Defaults to insert-only: a key
    /// already present for the destination parent, even with a null value, is
    /// left unchanged. Applies to every rule.
    #[serde(default)]
    #[schema(default = false)]
    pub overwrite: bool,
    /// Independent enrichment rules; must not be empty. Destination keys must
    /// not overlap within an attribute table, so rule order carries no
    /// precedence.
    pub lookups: Vec<AttributeLookupRule>,
}

/// One indexed lookup and the attributes to populate from its matched row.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributeLookupRule {
    /// UUID of a configured lookup table. Names are not unique identifiers.
    pub lookup_table: Uuid,
    /// Destination attribute table and default location of input attributes.
    /// `record` means log, span or metric attributes; it does not include metric
    /// data-point attributes. All outputs belong to this table.
    pub table: AttributeTable,
    /// Further record-selection predicate, ANDed with the top-level `when`.
    /// Omitted adds no restriction. A predicate error skips only this rule.
    ///
    /// A record destination may reference record, resource or scope attributes;
    /// a resource or scope destination only its own table — the same
    /// restriction the inherited top-level predicate carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<AttributeFilter>,
    /// Input attributes supplying the table's hash key or partition/range key.
    #[serde(rename = "match")]
    pub match_by: AttributeLookupMatch,
    /// Destination attribute key -> lookup-table column name. All columns come
    /// from the same matched row, so extra outputs add no index probes. Values
    /// keep their column types; IP values are emitted as strings.
    pub output: BTreeMap<String, String>,
}

/// Bind attributes to the index already defined by the lookup table.
///
/// A hash table requires exactly its configured key columns and no `point`; a
/// range table its partition columns (possibly none) plus `point`. Bindings are
/// named, so YAML map order does not determine tuple order.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributeLookupMatch {
    /// Lookup-table key/partition column -> input attribute. Empty only for an
    /// unpartitioned range lookup. Range bound columns are not probe bindings.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, AttributeLookupInput>,
    /// Point to search for in a range index; absent for a hash lookup. An
    /// explicit cast must be `integer` or `ip`, matching the table's range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<AttributeLookupInput>,
}

/// One exact attribute key supplying a probe input.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributeLookupInput {
    /// Defaults to the rule's destination table. A record rule may also read a
    /// resource attribute, resolved through the record's resource ID. No other
    /// cross-table read is supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<AttributeTable>,
    /// Dotted attribute key, matched byte for byte. A flattened array element
    /// is addressed by its literal index, such as `interfaces.0.ifIndex`; no
    /// segment carries a special meaning. Composite inputs align by parent ID,
    /// never by Arrow row position.
    pub key: String,
    /// Explicit conversion before probing. Without a cast the input must
    /// already match the table's key type. A conversion failure skips the
    /// probe; composite keys are never implicitly stringified.
    ///
    /// An `ip` key column or range point always needs `cast: ip`, because the
    /// input arrives as a string and would otherwise miss every record. A
    /// missing or contradicting cast is rejected when the table resolves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cast: Option<AttributeLookupCast>,
}

/// Supported key conversions. Doubles are excluded because the lookup indexes
/// do not accept floating-point keys; output columns may still contain them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttributeLookupCast {
    Bool,
    Integer,
    String,
    Ip,
}

impl AttributeLookup {
    /// Check the table-independent config contract. Resolving a loaded lookup
    /// table and validating its schema are separate processor-planning steps.
    pub fn validate(&self) -> Result<()> {
        if self.lookups.is_empty() {
            bail!("lookups must not be empty");
        }
        let mut destinations: FxHashSet<(&AttributeTable, &str)> = FxHashSet::default();
        for (i, rule) in self.lookups.iter().enumerate() {
            rule.validate().with_context(|| format!("lookups[{i}]"))?;
            if let Some(filter) = &self.when {
                validate_when(filter, &rule.table)
                    .with_context(|| format!("top-level when for lookups[{i}]"))?;
            }
            for key in rule.output.keys() {
                if !destinations.insert((&rule.table, key)) {
                    bail!(
                        "lookups[{i}].output destination {key:?} is written twice in {}",
                        rule.table.as_str()
                    );
                }
            }
        }
        Ok(())
    }
}

impl AttributeLookupRule {
    fn validate(&self) -> Result<()> {
        if let Some(filter) = &self.when {
            validate_when(filter, &self.table).context("when")?;
        }
        if self.match_by.keys.is_empty() && self.match_by.point.is_none() {
            bail!("match requires keys or a range point");
        }
        for column in self.match_by.keys.keys() {
            if column.trim().is_empty() {
                bail!("match.keys must not contain an empty column name");
            }
        }
        if let Some(point) = &self.match_by.point
            && matches!(
                point.cast,
                Some(AttributeLookupCast::Bool | AttributeLookupCast::String)
            )
        {
            bail!("match.point.cast must be integer or ip");
        }

        for input in self
            .match_by
            .keys
            .values()
            .chain(self.match_by.point.iter())
        {
            validate_key_path(&input.key)?;
            let source_table = input.table.as_ref().unwrap_or(&self.table);
            if source_table != &self.table
                && (self.table != AttributeTable::Record
                    || *source_table != AttributeTable::Resource)
            {
                bail!("cross-table inputs only support resource attributes in a record rule");
            }
        }

        if self.output.is_empty() {
            bail!("output must not be empty");
        }
        for (key, column) in &self.output {
            if column.trim().is_empty() {
                bail!("output column for {key:?} must not be empty");
            }
            validate_key_path(key)?;
        }
        Ok(())
    }
}

/// Reuse AttributeFilter's leaf validation while checking every branch for
/// shared-parent safety. This is a config-time walk, not predicate evaluation;
/// even a branch masked by another condition must use a permitted table.
pub(super) fn validate_when(filter: &AttributeFilter, destination: &AttributeTable) -> Result<()> {
    match filter {
        AttributeFilter::And(children) | AttributeFilter::Or(children) => {
            for child in children {
                validate_when(child, destination)?;
            }
        }
        AttributeFilter::Match(expr) => {
            expr.validate()?;
            if *destination != AttributeTable::Record && expr.table != *destination {
                bail!(
                    "predicates for {} may only reference that table, found {}",
                    destination.as_str(),
                    expr.table.as_str()
                );
            }
        }
    }
    Ok(())
}

/// Attribute keys are matched byte for byte, so this only rejects shapes that
/// can never name an attribute: empty keys and empty path segments.
pub(super) fn validate_key_path(key: &str) -> Result<()> {
    if key.trim().is_empty() || key.split('.').any(str::is_empty) {
        bail!("attribute key {key:?} must not be empty or contain empty path segments");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::processor::{Config, Processor};
    use serde_json::{Value, json};

    fn rule() -> Value {
        json!({
            "lookup_table": "019b77b3-df79-70a2-ab01-f43b2cf72402",
            "table": "record",
            "match": {"keys": {
                "device": {"table": "resource", "key": "host.id"},
                "index": {"key": "interface.index", "cast": "integer"}
            }},
            "output": {"interface.name": "name", "interface.speed": "speed"}
        })
    }

    fn config(rules: Vec<Value>) -> AttributeLookup {
        serde_json::from_value(json!({"lookups": rules})).unwrap()
    }

    fn predicate(table: &str) -> Value {
        json!({"match": {
            "table": table,
            "key": "environment",
            "key_is_regex": false,
            "op": "=",
            "value": {"string": "production"}
        }})
    }

    #[test]
    fn omitted_and_null_guards_add_no_restrictions() {
        for explicit_null in [false, true] {
            let mut input = json!({"lookups": [rule()]});
            if explicit_null {
                input["when"] = Value::Null;
                input["lookups"][0]["when"] = Value::Null;
            }
            let parsed: AttributeLookup = serde_json::from_value(input).unwrap();
            parsed.validate().unwrap();
            assert!(parsed.when.is_none());
            assert!(parsed.lookups[0].when.is_none());
            let output = serde_json::to_value(parsed).unwrap();
            assert!(output.get("when").is_none());
            assert!(output["lookups"][0].get("when").is_none());
        }
    }

    #[test]
    fn both_guards_round_trip_with_nested_existing_filters() {
        let mut input = json!({"when": predicate("resource"), "lookups": [rule()]});
        input["lookups"][0]["when"] = json!({"and": [
            predicate("record"), {"or": [predicate("scope"), predicate("resource")]}
        ]});
        let parsed: AttributeLookup = serde_json::from_value(input).unwrap();
        parsed.validate().unwrap();
        let yaml = serde_yaml_ng::to_string(&parsed).unwrap();
        let restored: AttributeLookup = serde_yaml_ng::from_str(&yaml).unwrap();
        restored.validate().unwrap();
        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            serde_json::to_value(restored).unwrap()
        );
    }

    #[test]
    fn guards_check_all_nested_tables_at_both_config_levels() {
        for destination in ["record", "resource", "scope"] {
            for source in ["record", "resource", "scope"] {
                for top_level in [false, true] {
                    let mut lookup = rule();
                    lookup["table"] = json!(destination);
                    lookup["match"]["keys"]["device"]["table"] = json!(destination);
                    let guard = json!({"or": [
                        predicate(destination), {"and": [predicate(source)]}
                    ]});
                    let mut input = json!({"lookups": [lookup]});
                    if top_level {
                        input["when"] = guard;
                    } else {
                        input["lookups"][0]["when"] = guard;
                    }
                    let parsed: AttributeLookup = serde_json::from_value(input).unwrap();
                    assert_eq!(
                        parsed.validate().is_ok(),
                        destination == "record" || destination == source,
                        "destination={destination}, source={source}, top_level={top_level}"
                    );
                }
            }
        }
    }

    #[test]
    fn inherited_guard_is_checked_against_every_destination() {
        let mut resource_rule = rule();
        resource_rule["table"] = json!("resource");
        let input = json!({"when": predicate("record"), "lookups": [rule(), resource_rule]});
        let parsed: AttributeLookup = serde_json::from_value(input).unwrap();
        let error = format!("{:#}", parsed.validate().unwrap_err());
        assert!(error.contains("top-level when for lookups[1]"), "{error}");
        assert!(error.contains("resource_attrs"), "{error}");
    }

    #[test]
    fn guards_reuse_existing_filter_validation() {
        let mut invalid = predicate("record");
        invalid["match"]["op"] = json!("=~");
        invalid["match"]["value"] = json!({"integer": 42});
        for top_level in [false, true] {
            let mut input = json!({"lookups": [rule()]});
            let guard = json!({"and": [{"or": [invalid.clone()]}]});
            if top_level {
                input["when"] = guard;
            } else {
                input["lookups"][0]["when"] = guard;
            }
            let parsed: AttributeLookup = serde_json::from_value(input).unwrap();
            let error = format!("{:#}", parsed.validate().unwrap_err());
            assert!(error.contains("when"), "{error}");
            assert!(
                error.contains("Regex operations are not supported"),
                "{error}"
            );
        }
    }

    #[test]
    fn composite_inputs_and_resource_broadcast_round_trip() {
        let original = config(vec![rule()]);
        original.validate().unwrap();
        assert!(!original.overwrite);
        let yaml = serde_yaml_ng::to_string(&original).unwrap();
        let restored: AttributeLookup = serde_yaml_ng::from_str(&yaml).unwrap();
        restored.validate().unwrap();
        assert_eq!(
            serde_json::to_value(original).unwrap(),
            serde_json::to_value(restored).unwrap()
        );
    }

    #[test]
    fn scalar_and_partitioned_range_lookups_are_supported() {
        let mut scalar = rule();
        scalar["match"] = json!({"keys": {"host": {"key": "host.name"}}});
        scalar["output"] = json!({"host.owner": "owner"});
        let mut range = rule();
        range["match"] = json!({
            "keys": {"vrf": {"key": "vrf"}},
            "point": {"key": "flow.ip", "cast": "ip"}
        });
        range["output"] = json!({"flow.country": "country"});
        config(vec![scalar, range.clone()]).validate().unwrap();
        range["match"].as_object_mut().unwrap().remove("keys");
        config(vec![range]).validate().unwrap();
    }

    /// `*` used to bind an array index. It is an ordinary key character now,
    /// so nothing about it is validated or expanded.
    #[test]
    fn a_star_is_an_ordinary_key_character() {
        let mut plain = rule();
        plain["match"] = json!({"keys": {"a": {"key": "left.*.id"}, "b": {"key": "right.*.id"}}});
        plain["output"] = json!({"left.*.name": "name", "name": "speed"});
        config(vec![plain]).validate().unwrap();
    }

    #[test]
    fn unsupported_cross_table_inputs_are_rejected() {
        for (destination, source, key) in [
            ("resource", "record", "host.id"),
            ("scope", "resource", "host.id"),
            ("record", "scope", "host.id"),
        ] {
            let mut invalid = rule();
            invalid["table"] = json!(destination);
            invalid["match"]["keys"]["device"] = json!({"table": source, "key": key});
            assert!(config(vec![invalid]).validate().is_err());
        }
    }

    #[test]
    fn duplicate_output_destinations_are_rejected_per_table() {
        assert!(config(vec![rule(), rule()]).validate().is_err());
        let mut other = rule();
        other["match"] = json!({"keys": {"id": {"key": "interface.id"}}});
        other["output"] = json!({"interface.name": "name"});
        assert!(config(vec![rule(), other.clone()]).validate().is_err());
        other["table"] = json!("resource");
        config(vec![rule(), other.clone()]).validate().unwrap();
        other["table"] = json!("record");
        other["output"] = json!({"interface.alias": "name"});
        config(vec![rule(), other]).validate().unwrap();
    }

    #[test]
    fn empty_bindings_invalid_paths_and_invalid_point_casts_are_rejected() {
        assert!(config(vec![]).validate().is_err());
        for (field, replacement) in [
            ("match", json!({})),
            ("match", json!({"keys": {"": {"key": "id"}}})),
            ("match", json!({"point": {"key": "id", "cast": "string"}})),
            ("output", json!({})),
            ("output", json!({"interface.name": " "})),
        ] {
            let mut invalid = rule();
            invalid[field] = replacement;
            assert!(config(vec![invalid]).validate().is_err());
        }
        for key in ["", " ", "a..b", ".a", "a."] {
            let mut invalid = rule();
            invalid["match"]["keys"]["index"]["key"] = json!(key);
            assert!(config(vec![invalid]).validate().is_err(), "{key:?}");
            let mut invalid = rule();
            invalid["output"] = json!({key: "name"});
            assert!(config(vec![invalid]).validate().is_err(), "{key:?}");
        }
    }

    #[test]
    fn unknown_options_and_floating_point_key_casts_are_not_accepted() {
        let mut invalid = rule();
        invalid["match"]["keys"]["index"]["cast"] = json!("double");
        assert!(serde_json::from_value::<AttributeLookupRule>(invalid).is_err());
        let mut invalid = rule();
        invalid["outputs"] = json!({"ignored.name": "name"});
        assert!(serde_json::from_value::<AttributeLookupRule>(invalid).is_err());
    }

    #[test]
    fn the_documented_config_deserializes_validates_and_loads_locally() {
        let reference = include_str!("../../../config-full.yaml");
        let example = reference
            .split_once("  # BEGIN attribute_lookup example\n")
            .unwrap()
            .1
            .split_once("  # END attribute_lookup example")
            .unwrap()
            .0;
        let yaml = example
            .lines()
            .map(|line| line.strip_prefix("  # ").unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        // Through `Processor` rather than a bespoke struct: that exercises the
        // `type: attribute_lookup` tag on `Processor` alongside the config
        // body, and lets YAML's !match / !string tags reach the real enums
        // instead of passing through untagged JSON.
        let mut processors: Vec<Processor> = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(processors.len(), 1);
        processors[0].validate().unwrap();
        let Config::AttributeLookup(config) = &processors[0].config else {
            panic!("the documented type tag must select AttributeLookup");
        };
        assert_eq!(config.lookups.len(), 2);
        assert!(config.when.is_some());
        assert!(config.lookups[0].when.is_none());
        assert!(config.lookups[1].when.is_some());

        // Local mode validates its processors too, so the documented block has to
        // survive the same path a YAML deployment takes. `global.validate`
        // creates the disk-channel directory, hence the temporary one.
        let dir = tempfile::tempdir().unwrap();
        let mut local: crate::config::LocalFileConfig = serde_yaml_ng::from_str(&format!(
            "global:\n  disk_channel:\n    data_dir: {}\nprocessors:\n{yaml}",
            dir.path().display()
        ))
        .unwrap();
        local.validate().unwrap();
        assert!(matches!(
            local.processors[0].config,
            Config::AttributeLookup(_)
        ));
    }

    #[test]
    fn openapi_exposes_the_lookup_config_and_its_field_comments() {
        use utoipa::OpenApi;
        let api = serde_json::to_value(crate::api::ApiDoc::openapi()).unwrap();
        let schemas = &api["components"]["schemas"];
        let lookup = &schemas["AttributeLookup"];
        assert_eq!(lookup["properties"]["overwrite"]["default"], json!(false));
        // Presence, not wording, as `attribute_collect`'s twin does: the point
        // is that the doc comments reach the schema, and a phrase match breaks
        // on any edit to them.
        assert!(!lookup["description"].as_str().unwrap().is_empty());
        for (field, property) in lookup["properties"].as_object().unwrap() {
            // An optional referenced type keeps its comment on the non-null
            // branch of the oneOf, not on the surrounding property.
            let description = property.get("description").or_else(|| {
                property
                    .get("oneOf")?
                    .as_array()?
                    .iter()
                    .find_map(|variant| variant.get("description"))
            });
            assert!(
                description
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| !s.is_empty()),
                "AttributeLookup.{field}"
            );
        }
        let rule = &schemas["AttributeLookupRule"]["properties"];
        assert!(rule.get("match").is_some());
        assert!(rule.get("match_by").is_none());
        for name in ["AttributeLookup", "AttributeLookupRule"] {
            assert!(schemas[name]["properties"].get("when").is_some());
            assert!(
                !schemas[name]["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("when"))
            );
        }
        assert!(schemas.get("AttributeLookupInput").is_some());
        assert!(schemas.get("AttributeLookupCast").is_some());
    }
}
