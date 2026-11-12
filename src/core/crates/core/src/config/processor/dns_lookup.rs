// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Configuration for DNS enrichment; `processor::dns_lookup` executes it
//! against `cache::dns`, which `global.dns` sizes and its resolver fills.

use super::attribute_lookup::{
    AttributeLookupCast, AttributeLookupInput, validate_key_path, validate_when,
};
use super::{AttributeFilter, AttributeTable};
use anyhow::{Context, Result, bail};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Enrich attributes with DNS answers.
///
/// A lookup never waits for DNS: it reads the node's DNS cache, and an address
/// that is not cached yet (or whose answer has expired) is queued for the
/// resolver and left unenriched until a later record finds the answer. The
/// first records of a new address therefore pass through unchanged.
///
/// Missing inputs, values that are not addresses, cache misses and negative
/// answers leave the destination unchanged. Every rule reads the incoming
/// attributes: another rule's output is not visible as an input.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DnsLookup {
    /// Record-selection predicate over incoming attributes, evaluated once for
    /// all rules and ANDed with each rule's own `when`. Omitted means every
    /// record is eligible; unselected records pass through unchanged.
    ///
    /// For a resource or scope destination every predicate leaf must reference
    /// that same table; a record destination may reference any table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<AttributeFilter>,
    /// Replace existing destination values. Defaults to insert-only: a key
    /// already present for the destination parent is left unchanged.
    #[serde(default)]
    #[schema(default = false)]
    pub overwrite: bool,
    /// Independent lookups; must not be empty. Destination keys must not
    /// overlap within an attribute table.
    pub lookups: Vec<DnsLookupRule>,
}

/// One question per record and the attribute its answer is written to.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DnsLookupRule {
    /// What is asked about the input.
    pub record: DnsRecordType,
    /// Destination attribute table and default location of the input
    /// attribute. `record` means log, span or metric attributes.
    pub table: AttributeTable,
    /// Further record-selection predicate, ANDed with the top-level `when`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<AttributeFilter>,
    /// The attribute holding the address: one key, fallback keys tried in
    /// order, or a regex capture of the value. Always read as an IP (text, or
    /// the bare-integer form), so `cast` may be left out; if set it must be
    /// `ip`. A record rule may read a resource attribute.
    pub input: AttributeLookupInput,
    /// Destination attribute key; the answer is written as a string.
    pub output: String,
}

/// DNS record types a rule can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DnsRecordType {
    /// Reverse lookup: the name of an IPv4 or IPv6 address, without the
    /// trailing dot. IPv4-mapped IPv6 addresses are asked as IPv4.
    Ptr,
}

impl DnsLookup {
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
            if !destinations.insert((&rule.table, &rule.output)) {
                bail!(
                    "lookups[{i}].output {:?} is written twice in {}",
                    rule.output,
                    rule.table.as_str()
                );
            }
        }
        Ok(())
    }
}

impl DnsLookupRule {
    fn validate(&self) -> Result<()> {
        if let Some(filter) = &self.when {
            validate_when(filter, &self.table).context("when")?;
        }
        self.input.key.validate().context("input")?;
        if self
            .input
            .cast
            .is_some_and(|cast| cast != AttributeLookupCast::Ip)
        {
            bail!("input.cast must be ip, or left out: the input is always an address");
        }
        let source_table = self.input.table.as_ref().unwrap_or(&self.table);
        if source_table != &self.table
            && (self.table != AttributeTable::Record || *source_table != AttributeTable::Resource)
        {
            bail!("cross-table inputs only support resource attributes in a record rule");
        }
        validate_key_path(&self.output).context("output")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::processor::{Config, Processor};
    use serde_json::json;

    fn config(rules: serde_json::Value) -> Result<DnsLookup> {
        let config: DnsLookup = serde_json::from_value(json!({ "lookups": rules }))?;
        config.validate()?;
        Ok(config)
    }

    fn rule() -> serde_json::Value {
        json!({
            "record": "ptr",
            "table": "record",
            "input": {"key": "source.address"},
            "output": "source.domain",
        })
    }

    #[test]
    fn a_rule_reads_one_address_and_writes_one_name() {
        let parsed = config(json!([rule()])).unwrap();
        assert!(!parsed.overwrite);
        assert_eq!(parsed.lookups[0].record, DnsRecordType::Ptr);
        // the same input forms as attribute_lookup: fallbacks and extraction
        let mut fallback = rule();
        fallback["input"]["key"] = json!(["src4", "src6"]);
        fallback["input"]["cast"] = json!("ip");
        let mut extract = rule();
        extract["input"]["key"] = json!({"from": "peer", "pattern": "^([^:]+)"});
        extract["output"] = json!("peer.domain");
        config(json!([fallback, extract])).unwrap();
    }

    #[test]
    fn invalid_rules_are_rejected() {
        assert!(config(json!([])).is_err());
        for (field, value) in [
            ("record", json!("a")),
            ("output", json!("a..b")),
            ("output", json!("")),
        ] {
            let mut invalid = rule();
            invalid[field] = value;
            assert!(config(json!([invalid])).is_err(), "{field}");
        }
        let mut cast = rule();
        cast["input"]["cast"] = json!("string");
        assert!(config(json!([cast])).is_err());
        let mut cross = rule();
        cross["table"] = json!("scope");
        cross["input"]["table"] = json!("resource");
        assert!(config(json!([cross])).is_err());
        // two rules writing one destination
        assert!(config(json!([rule(), rule()])).is_err());
        let mut elsewhere = rule();
        elsewhere["table"] = json!("resource");
        config(json!([rule(), elsewhere])).unwrap();
        let mut unknown = rule();
        unknown["ttl"] = json!(5);
        assert!(config(json!([unknown])).is_err());
    }

    #[test]
    fn the_documented_config_deserializes_and_validates() {
        let reference = include_str!("../../../../../config-full.yaml");
        let example = reference
            .split_once("  # BEGIN dns_lookup example\n")
            .unwrap()
            .1
            .split_once("  # END dns_lookup example")
            .unwrap()
            .0;
        let yaml = example
            .lines()
            .map(|line| line.strip_prefix("  # ").unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let mut processors: Vec<Processor> = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(processors.len(), 1);
        processors[0].validate().unwrap();
        let Config::DnsLookup(config) = &processors[0].config else {
            panic!("the documented type tag must select DnsLookup");
        };
        assert_eq!(config.lookups.len(), 2);
    }
}
