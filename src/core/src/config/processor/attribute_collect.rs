// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Configuration for collecting observed attributes into mutable lookup tables;
//! `processor::attribute_collect` executes it. Validation here is
//! table-independent: mutability, complete column bindings, types and key
//! compatibility are checked per batch, because a table config edit swaps the
//! table out from under a running processor.

use super::attribute_lookup::{validate_key_path, validate_when};
use super::{AttributeFilter, AttributeTable};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;
use uuid::Uuid;

/// Collect complete rows from incoming attributes into mutable lookup tables.
///
/// Each rule produces one row per source parent and upserts it under the
/// target table's configured key. An existing row is replaced in full and its
/// TTL refreshed, never merged. TTL, capacity and key definitions belong to the
/// lookup table.
///
/// Missing inputs, null keys or bounds and failed conversions skip the whole
/// candidate row and count an error; an explicit null non-key value is stored
/// as null, and capacity rejections are counted separately. Telemetry always
/// passes through unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributeCollect {
    /// Record-selection predicate over incoming attributes, evaluated once for
    /// all rules and ANDed with each rule's own `when`. Omitted means every
    /// record is eligible. A predicate error skips all collection without
    /// dropping telemetry.
    ///
    /// For a resource or scope rule every predicate leaf must reference that
    /// rule's source table, so selection is uniform for its shared parent; a
    /// record rule may reference any table. Selection is per record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<AttributeFilter>,
    /// Collection rules; must not be empty. All read incoming attributes and
    /// may target the same lookup table. A repeated key replaces the whole row
    /// in write order, which concurrent writers do not order by event time.
    pub collections: Vec<AttributeCollectRule>,
}

/// One mutable lookup table and the source attributes that populate its rows.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributeCollectRule {
    /// UUID of a configured lookup table whose source is `processor` (mutable);
    /// file-backed tables are not writable. Names are not unique identifiers.
    pub lookup_table: Uuid,
    /// Source attribute table and default location of column inputs. `record`
    /// means log, span or metric attributes, excluding data-point attributes.
    /// A rule produces one row per eligible parent.
    pub table: AttributeTable,
    /// Further record-selection predicate, ANDed with the top-level `when`.
    /// Omitted adds no restriction. A predicate error skips only this rule.
    ///
    /// A record rule may reference record, resource or scope attributes; a
    /// resource or scope rule only its own table, inherited guard included.
    /// Selection is over whole records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<AttributeFilter>,
    /// Bind columns to the elements of one flattened array by a naming field
    /// rather than by element position. Required by, and only useful with,
    /// bindings in `columns` that have no `key`. Elements are read from the
    /// rule's source table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<AttributeCollectPivot>,
    /// Lookup-table column name -> source binding. Must bind every declared
    /// column exactly once, hash keys and range partition/start/end columns
    /// included; an unknown column is rejected when the table resolves. YAML
    /// map order determines neither schema nor composite-key order.
    ///
    /// A binding with a `key` reads that exact attribute. A `null` binding, or
    /// one without `key`, is *pivoted*: it reads the `pivot.value` field of the
    /// array element whose normalized name equals the column name (or the
    /// binding's `name`), and requires `pivot` on the rule.
    ///
    /// A hash key is taken from the resulting row. In a range table the bound
    /// columns supply the interval, whose type and inclusive ends the table
    /// defines; bounds are consumed by the index, not stored as readable
    /// values. There is no match block and no per-processor key, TTL or
    /// capacity setting.
    ///
    /// A missing input or failed cast skips the whole candidate row, leaving
    /// any existing row untouched. No partial updates, no fallback values.
    pub columns: BTreeMap<String, AttributeCollectBinding>,
}

/// One entry of `columns`: `null` for a pivoted binding with every option
/// defaulted, otherwise an [`AttributeCollectInput`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
pub struct AttributeCollectBinding(pub Option<AttributeCollectInput>);

impl std::ops::Deref for AttributeCollectBinding {
    type Target = Option<AttributeCollectInput>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Turn the elements of one flattened array into named columns.
///
/// OTAP flattens an array of maps into `<array>.<n>.<field>` attributes, where
/// `<n>` is the element's position. Position is not a stable identity — the SNMP
/// walk, for instance, appends values in completion order and skips absent
/// ones — so a pivoted column is found by the value of one sibling field (the
/// `name`) and read from another (the `value`). With `array: snmp.data`,
/// `name: oid_name`, `value: value_formatted` and the `snmp_object` format, the
/// element `snmp.data.3.oid_name = "IF-MIB::ifDescr.33"` supplies column
/// `ifDescr` from `snmp.data.3.value_formatted`.
///
/// Every pivoted column of a rule is looked up in the same array. Two elements
/// of one record that normalize to the same name resolve to the highest `<n>`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributeCollectPivot {
    /// Dotted key of the flattened array, such as `snmp.data`; its elements are
    /// the attributes `<array>.<n>.<field>` for a nonnegative decimal `<n>`.
    pub array: String,
    /// Element field whose string value names the column, such as `oid_name`.
    /// An element whose name field is missing, not a string, or normalizes to
    /// nothing binds no column.
    pub name: String,
    /// Element field supplying the cell unless the binding overrides it with
    /// its own `value`, such as `value_formatted`.
    pub value: String,
    /// How a raw name becomes the name matched against the columns. Omitted
    /// uses the raw name verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_format: Option<NameFormat>,
}

/// Normalization applied to a pivot element's raw name before it is matched to
/// a column. A raw name that does not fit the format binds no column.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum NameFormat {
    /// The object name out of an OID as the MIB renders it,
    /// `MODULE::object.instance`: `IF-MIB::ifDescr.33` -> `ifDescr`. Cheaper
    /// than the equivalent regular expression.
    SnmpObject(SnmpObjectFormat),
    /// One capture group of a regular expression, for names in any other shape.
    RegexExtract(RegexExtractFormat),
}

/// Options of the `snmp_object` name format.
///
/// The `MODULE::` prefix, when present, is always dropped. A raw name without a
/// MIB match — a bare numeric OID such as `1.3.6.1.2.1.2.2.1.2.33` — comes out
/// as its first arc (`1`) and so matches no column, which is the intent.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnmpObjectFormat {
    /// Also drop the instance suffix after the object name: `ifDescr.33` ->
    /// `ifDescr`, `sysName.0` -> `sysName`. With `false` the column is named
    /// `ifDescr.33`. Defaults to `true`: for a table walked in `table_format`
    /// the instance is already carried by the row's `snmp.id`.
    #[serde(default = "default_true")]
    #[schema(default = true)]
    pub strip_instance: bool,
}

/// Options of the `regex_extract` name format.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RegexExtractFormat {
    /// Regular expression in Rust `regex` syntax, searched anywhere in the raw
    /// name; the first match is used. Invalid syntax is rejected when the
    /// config is read.
    #[schema(value_type = String)]
    pub pattern: RegexPattern,
    /// Capture group whose text becomes the name, `0` being the whole match.
    /// Must exist in `pattern`. An unmatched or empty group binds no column.
    /// Defaults to `1`.
    #[serde(default = "default_group")]
    #[schema(default = 1)]
    pub group: usize,
}

/// A regular expression compiled once when the config is read, serialized as
/// its pattern string. Not a schema component of its own: the field using it
/// declares itself a string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RegexPattern(pub Regex);

impl TryFrom<String> for RegexPattern {
    type Error = regex::Error;

    fn try_from(pattern: String) -> Result<Self, Self::Error> {
        Regex::new(&pattern).map(Self)
    }
}

impl From<RegexPattern> for String {
    fn from(pattern: RegexPattern) -> Self {
        pattern.0.as_str().to_owned()
    }
}

/// One column's source: an exact attribute key, or — without `key` and with a
/// `pivot` on the rule — the field of an array element selected by name.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributeCollectInput {
    /// Only with `key`. Defaults to the rule's source table. A record rule may
    /// also read a resource attribute through the record's resource ID. No
    /// other cross-table read is supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<AttributeTable>,
    /// Dotted attribute key, matched byte for byte. A flattened array element
    /// is addressed by its literal index, such as `interfaces.0.ifIndex`; no
    /// segment carries a special meaning. Column inputs align by parent ID,
    /// never by Arrow row position; the stored key comes from the configured
    /// key-column values.
    ///
    /// Omitted, the binding is pivoted and the rule must set `pivot`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Pivoted bindings only: the element field supplying the cell instead of
    /// `pivot.value`, for instance `value` to store the raw SNMP value where
    /// the default is the MIB-formatted one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Pivoted bindings only: the normalized element name to bind when it
    /// differs from the lookup-table column name, so a column `description`
    /// can be fed by the object `ifDescr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Explicit conversion before writing. Without a cast the input must
    /// already match the column's declared type. A conversion failure skips the
    /// whole candidate row; there is no implicit stringification.
    ///
    /// `double` is allowed for a value column, never a hash or partition key.
    /// A range bound must use `integer` or `ip`, matching the table's range.
    ///
    /// An `ip` column always needs `cast: ip`, bound or not, because the input
    /// arrives as a string and would otherwise fail the column's type check and
    /// skip the row. A missing or contradicting cast is rejected when the table
    /// resolves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cast: Option<AttributeCollectCast>,
}

/// Supported column conversions. Unlike lookup probe casts, this includes
/// doubles because collection also supplies non-key value columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttributeCollectCast {
    Bool,
    /// Not valid for index keys or bounds.
    Double,
    Integer,
    String,
    Ip,
}

fn default_true() -> bool {
    true
}

fn default_group() -> usize {
    1
}

impl AttributeCollect {
    /// Validate the table-independent contract; everything schema-dependent
    /// waits for table resolution.
    pub fn validate(&self) -> Result<()> {
        if self.collections.is_empty() {
            bail!("collections must not be empty");
        }
        for (i, rule) in self.collections.iter().enumerate() {
            rule.validate()
                .with_context(|| format!("collections[{i}]"))?;
            if let Some(filter) = &self.when {
                validate_when(filter, &rule.table)
                    .with_context(|| format!("top-level when for collections[{i}]"))?;
            }
        }
        Ok(())
    }
}

impl AttributeCollectRule {
    fn validate(&self) -> Result<()> {
        if let Some(filter) = &self.when {
            validate_when(filter, &self.table).context("when")?;
        }
        if self.columns.is_empty() {
            bail!("columns must not be empty");
        }
        if let Some(pivot) = &self.pivot {
            pivot.validate().context("pivot")?;
        }
        let mut pivoted = 0usize;
        for (column, input) in &self.columns {
            if column.trim().is_empty() {
                bail!("columns must not contain an empty column name");
            }
            let input = input.0.as_ref();
            match input.and_then(|input| input.key.as_deref()) {
                Some(key) => {
                    let input = input.expect("a key comes from a binding");
                    validate_key_path(key).with_context(|| format!("columns[{column:?}].key"))?;
                    if input.value.is_some() || input.name.is_some() {
                        bail!(
                            "columns[{column:?}]: `value` and `name` apply only to pivoted bindings, which have no `key`"
                        );
                    }
                    let source_table = input.table.as_ref().unwrap_or(&self.table);
                    if source_table != &self.table
                        && (self.table != AttributeTable::Record
                            || *source_table != AttributeTable::Resource)
                    {
                        bail!(
                            "cross-table inputs only support resource attributes in a record rule"
                        );
                    }
                }
                None => {
                    if self.pivot.is_none() {
                        bail!(
                            "columns[{column:?}] has no `key`, which requires `pivot` on the rule"
                        );
                    }
                    pivoted += 1;
                    if let Some(input) = input {
                        if input.table.is_some() {
                            bail!(
                                "columns[{column:?}].table applies only to bindings with a `key`"
                            );
                        }
                        for (field, text) in [("value", &input.value), ("name", &input.name)] {
                            if text.as_deref().is_some_and(|t| t.trim().is_empty()) {
                                bail!("columns[{column:?}].{field} must not be empty");
                            }
                        }
                    }
                }
            }
        }
        if self.pivot.is_some() && pivoted == 0 {
            bail!("pivot is configured but no column uses it (a pivoted binding has no `key`)");
        }
        Ok(())
    }
}

impl AttributeCollectPivot {
    fn validate(&self) -> Result<()> {
        validate_key_path(&self.array).context("array")?;
        for (field, text) in [("name", &self.name), ("value", &self.value)] {
            validate_key_path(text).context(field)?;
        }
        if let Some(NameFormat::RegexExtract(format)) = &self.name_format {
            format.validate().context("name_format")?;
        }
        Ok(())
    }
}

impl RegexExtractFormat {
    fn validate(&self) -> Result<()> {
        // `captures_len` counts the implicit whole-match group 0.
        let groups = self.pattern.0.captures_len();
        if self.group >= groups {
            bail!(
                "group {} does not exist: pattern {:?} has {} capture group(s)",
                self.group,
                self.pattern.0.as_str(),
                groups - 1
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::processor::{Config, Processor};
    use serde_json::{Value, json};

    fn rule() -> Value {
        json!({
            "lookup_table": "019b77b3-df79-70a2-ab01-f43b2cf72403",
            "table": "record",
            "columns": {
                "device": {"table": "resource", "key": "host.id"},
                "index": {"key": "interface.ifIndex", "cast": "integer"},
                "name": {"key": "interface.ifName"},
                "speed": {"key": "interface.ifSpeed", "cast": "double"}
            }
        })
    }

    /// The SNMP interface-table shape the pivot exists for: one exact key plus
    /// pivoted bindings in every spelling.
    fn pivot_rule() -> Value {
        json!({
            "lookup_table": "019b77b3-df79-70a2-ab01-f43b2cf72403",
            "table": "record",
            "pivot": {
                "array": "snmp.data",
                "name": "oid_name",
                "value": "value_formatted",
                "name_format": {"type": "snmp_object", "strip_instance": true}
            },
            "columns": {
                "ifIndex": {"key": "snmp.id", "cast": "integer"},
                "ifDescr": null,
                "ifAlias": {"cast": "string"},
                "ifType": {"value": "value", "cast": "integer"},
                "admin_status": {"name": "ifAdminStatus"}
            }
        })
    }

    fn config(rules: Vec<Value>) -> AttributeCollect {
        serde_json::from_value(json!({"collections": rules})).unwrap()
    }

    fn predicate(table: &str) -> Value {
        json!({"match": {
            "table": table, "key": "environment", "key_is_regex": false,
            "op": "=", "value": {"string": "production"}
        }})
    }

    #[test]
    fn inputs_resource_broadcast_and_nested_guards_round_trip() {
        let mut input = json!({"when": predicate("resource"), "collections": [rule()]});
        input["collections"][0]["when"] = json!({"and": [
            predicate("record"), {"or": [predicate("scope"), predicate("resource")]}
        ]});
        let parsed: AttributeCollect = serde_json::from_value(input.clone()).unwrap();
        parsed.validate().unwrap();
        let yaml = serde_yaml_ng::to_string(&parsed).unwrap();
        let restored: AttributeCollect = serde_yaml_ng::from_str(&yaml).unwrap();
        restored.validate().unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), input);
    }

    #[test]
    fn omitted_and_null_options_have_the_same_defaults() {
        for explicit_null in [false, true] {
            let mut input = json!({"collections": [rule()]});
            if explicit_null {
                input["when"] = Value::Null;
                input["collections"][0]["when"] = Value::Null;
                input["collections"][0]["pivot"] = Value::Null;
                input["collections"][0]["columns"]["name"]["table"] = Value::Null;
                input["collections"][0]["columns"]["name"]["cast"] = Value::Null;
                input["collections"][0]["columns"]["name"]["value"] = Value::Null;
                input["collections"][0]["columns"]["name"]["name"] = Value::Null;
            }
            let parsed: AttributeCollect = serde_json::from_value(input).unwrap();
            parsed.validate().unwrap();
            assert_eq!(
                serde_json::to_value(parsed).unwrap(),
                json!({"collections": [rule()]})
            );
        }
    }

    #[test]
    fn exact_index_star_and_range_column_bindings_are_supported() {
        let mut scalar = rule();
        // `*` is an ordinary key character with no special meaning.
        scalar["columns"] = json!({
            "id": {"key": "sessions.0.id"}, "value": {"key": "sessions.*.value"}
        });
        let mut range = rule();
        range["columns"] = json!({
            "vrf": {"key": "vrf"},
            "start": {"key": "range.start", "cast": "ip"},
            "end": {"key": "range.end", "cast": "ip"},
            "owner": {"key": "range.owner"}
        });
        // Repeated targets are legal: each rule contributes complete rows.
        config(vec![scalar, range.clone(), range])
            .validate()
            .unwrap();
        for table in ["record", "resource", "scope"] {
            let mut local = rule();
            local["table"] = json!(table);
            local["columns"]["device"]["table"] = json!(table);
            config(vec![local]).validate().unwrap();
        }
    }

    #[test]
    fn invalid_paths_and_empty_bindings_are_rejected() {
        assert!(config(vec![]).validate().is_err());
        for columns in [json!({}), json!({" ": {"key": "id"}})] {
            let mut invalid = rule();
            invalid["columns"] = columns;
            assert!(config(vec![invalid]).validate().is_err());
        }
        for key in ["", " ", "a..b", ".a", "a."] {
            let mut invalid = rule();
            invalid["columns"]["name"]["key"] = json!(key);
            assert!(config(vec![invalid]).validate().is_err(), "{key:?}");
        }
    }

    #[test]
    fn unsupported_cross_table_inputs_are_rejected() {
        for (table, source, key) in [
            ("resource", "record", "id"),
            ("scope", "resource", "id"),
            ("record", "scope", "id"),
        ] {
            let mut invalid = rule();
            invalid["table"] = json!(table);
            invalid["columns"]["device"] = json!({"table": source, "key": key});
            assert!(config(vec![invalid]).validate().is_err());
        }
    }

    #[test]
    fn guards_check_nested_tables_at_both_levels() {
        for table in ["record", "resource", "scope"] {
            for source in ["record", "resource", "scope"] {
                for top_level in [false, true] {
                    let mut local = rule();
                    local["table"] = json!(table);
                    local["columns"]["device"]["table"] = json!(table);
                    let guard = json!({"or": [predicate(table), {"and": [predicate(source)]}]});
                    let mut input = json!({"collections": [local]});
                    if top_level {
                        input["when"] = guard;
                    } else {
                        input["collections"][0]["when"] = guard;
                    }
                    let parsed: AttributeCollect = serde_json::from_value(input).unwrap();
                    assert_eq!(
                        parsed.validate().is_ok(),
                        table == "record" || table == source
                    );
                }
            }
        }
    }

    #[test]
    fn inherited_guards_are_checked_against_every_rule() {
        let mut resource = rule();
        resource["table"] = json!("resource");
        let parsed: AttributeCollect = serde_json::from_value(json!({
            "when": predicate("record"), "collections": [rule(), resource]
        }))
        .unwrap();
        let error = format!("{:#}", parsed.validate().unwrap_err());
        assert!(
            error.contains("top-level when for collections[1]"),
            "{error}"
        );
        assert!(error.contains("resource_attrs"), "{error}");
    }

    #[test]
    fn guards_reuse_filter_validation() {
        let mut invalid = predicate("record");
        invalid["match"]["op"] = json!("=~");
        invalid["match"]["value"] = json!({"integer": 42});
        for top_level in [false, true] {
            let mut input = json!({"collections": [rule()]});
            let guard = json!({"and": [{"or": [invalid.clone()]}]});
            if top_level {
                input["when"] = guard;
            } else {
                input["collections"][0]["when"] = guard;
            }
            let parsed: AttributeCollect = serde_json::from_value(input).unwrap();
            let error = format!("{:#}", parsed.validate().unwrap_err());
            assert!(
                error.contains("Regex operations are not supported"),
                "{error}"
            );
        }
    }

    #[test]
    fn casts_cover_value_types_and_unknown_options_are_rejected() {
        for cast in ["bool", "double", "integer", "string", "ip"] {
            let mut valid = rule();
            valid["columns"]["speed"]["cast"] = json!(cast);
            config(vec![valid]).validate().unwrap();
        }
        for path in [
            "/overwrite",
            "/collections/0/match",
            "/collections/0/columns/name/fallback",
            "/collections/1/pivot/bogus",
            "/collections/1/pivot/name_format/bogus",
        ] {
            let mut invalid = json!({"collections": [rule(), pivot_rule()]});
            let (parent, field) = path.rsplit_once('/').unwrap();
            invalid.pointer_mut(parent).unwrap()[field] = json!(true);
            assert!(
                serde_json::from_value::<AttributeCollect>(invalid).is_err(),
                "{path}"
            );
        }
        let mut invalid = rule();
        invalid["columns"]["speed"]["cast"] = json!("float");
        assert!(serde_json::from_value::<AttributeCollectRule>(invalid).is_err());
        let mut invalid = rule();
        invalid["lookup_table"] = json!("table-name");
        assert!(serde_json::from_value::<AttributeCollectRule>(invalid).is_err());
    }

    #[test]
    fn pivoted_bindings_round_trip_and_default_their_options() {
        let parsed = config(vec![pivot_rule()]);
        parsed.validate().unwrap();
        let yaml = serde_yaml_ng::to_string(&parsed).unwrap();
        let restored: AttributeCollect = serde_yaml_ng::from_str(&yaml).unwrap();
        restored.validate().unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            json!({"collections": [pivot_rule()]})
        );
        let columns = &restored.collections[0].columns;
        assert!(columns["ifDescr"].is_none());
        assert!(columns["ifAlias"].as_ref().unwrap().key.is_none());

        // `strip_instance` and `group` default when omitted.
        let mut defaults = pivot_rule();
        defaults["pivot"]["name_format"] = json!({"type": "snmp_object"});
        let parsed = config(vec![defaults]);
        parsed.validate().unwrap();
        assert!(matches!(
            parsed.collections[0].pivot.as_ref().unwrap().name_format,
            Some(NameFormat::SnmpObject(SnmpObjectFormat {
                strip_instance: true
            }))
        ));
        let mut regex = pivot_rule();
        regex["pivot"]["name_format"] = json!({"type": "regex_extract", "pattern": "::(\\w+)"});
        let parsed = config(vec![regex]);
        parsed.validate().unwrap();
        let Some(NameFormat::RegexExtract(format)) =
            &parsed.collections[0].pivot.as_ref().unwrap().name_format
        else {
            panic!("expected regex_extract");
        };
        assert_eq!(format.group, 1);
        assert_eq!(format.pattern.0.as_str(), "::(\\w+)");
        assert_eq!(
            serde_json::to_value(&parsed).unwrap()["collections"][0]["pivot"]["name_format"],
            json!({"type": "regex_extract", "pattern": "::(\\w+)", "group": 1})
        );
        // No format at all is legal: the raw name is the column name.
        let mut raw = pivot_rule();
        raw["pivot"].as_object_mut().unwrap().remove("name_format");
        config(vec![raw]).validate().unwrap();
    }

    #[test]
    fn pivot_misuse_is_rejected() {
        // A pivoted binding without a pivot on the rule.
        let mut no_pivot = rule();
        no_pivot["columns"]["name"] = Value::Null;
        assert!(config(vec![no_pivot]).validate().is_err());
        let mut no_pivot = rule();
        no_pivot["columns"]["name"] = json!({"cast": "string"});
        assert!(config(vec![no_pivot]).validate().is_err());
        // Pivot-only options on a keyed binding, and the key-only option on a
        // pivoted one.
        for binding in [
            json!({"key": "snmp.id", "value": "value"}),
            json!({"key": "snmp.id", "name": "ifDescr"}),
            json!({"table": "resource"}),
            json!({"table": "resource", "cast": "string"}),
            json!({"value": " "}),
            json!({"name": ""}),
        ] {
            let mut invalid = pivot_rule();
            invalid["columns"]["ifDescr"] = binding.clone();
            assert!(config(vec![invalid]).validate().is_err(), "{binding}");
        }
        // A pivot nobody uses.
        let mut unused = pivot_rule();
        unused["columns"] = json!({"ifIndex": {"key": "snmp.id", "cast": "integer"}});
        assert!(config(vec![unused]).validate().is_err());
        // Malformed pivot fields.
        for (field, text) in [
            ("array", ""),
            ("array", "a..b"),
            ("name", " "),
            ("value", ""),
        ] {
            let mut invalid = pivot_rule();
            invalid["pivot"][field] = json!(text);
            assert!(
                config(vec![invalid]).validate().is_err(),
                "{field}={text:?}"
            );
        }
    }

    #[test]
    fn regex_formats_are_checked_when_read_and_when_validated() {
        let mut bad_syntax = pivot_rule();
        bad_syntax["pivot"]["name_format"] = json!({"type": "regex_extract", "pattern": "("});
        assert!(
            serde_json::from_value::<AttributeCollectRule>(bad_syntax).is_err(),
            "invalid syntax fails at deserialization, before validate"
        );
        let mut no_such_group = pivot_rule();
        no_such_group["pivot"]["name_format"] =
            json!({"type": "regex_extract", "pattern": "::(\\w+)", "group": 2});
        let error = format!("{:#}", config(vec![no_such_group]).validate().unwrap_err());
        assert!(error.contains("group 2 does not exist"), "{error}");
        let mut whole_match = pivot_rule();
        whole_match["pivot"]["name_format"] =
            json!({"type": "regex_extract", "pattern": "if\\w+", "group": 0});
        config(vec![whole_match]).validate().unwrap();
        let mut unknown_format = pivot_rule();
        unknown_format["pivot"]["name_format"] = json!({"type": "upper_case"});
        assert!(serde_json::from_value::<AttributeCollectRule>(unknown_format).is_err());
    }

    #[test]
    fn the_documented_config_deserializes_validates_and_loads_locally() {
        let reference = include_str!("../../../config-full.yaml");
        let example = reference
            .split_once("  # BEGIN attribute_collect example\n")
            .unwrap()
            .1
            .split_once("  # END attribute_collect example")
            .unwrap()
            .0;
        let yaml = example
            .lines()
            .map(|line| line.strip_prefix("  # ").unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        // Through `Processor` rather than a bespoke struct: that exercises the
        // `type: attribute_collect` tag on `Processor` alongside the config
        // body, which is what actually enables the processor.
        let mut processors: Vec<Processor> = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(processors.len(), 1);
        processors[0].validate().unwrap();
        let Config::AttributeCollect(config) = &processors[0].config else {
            panic!("the documented type tag must select AttributeCollect");
        };
        assert_eq!(config.collections.len(), 1);
        assert!(config.when.is_some());
        assert!(config.collections[0].when.is_some());
        assert_eq!(config.collections[0].columns.len(), 5);
        let pivot = config.collections[0].pivot.as_ref().unwrap();
        assert_eq!(pivot.array, "snmp.data");
        assert!(matches!(
            pivot.name_format,
            Some(NameFormat::SnmpObject(SnmpObjectFormat {
                strip_instance: true
            }))
        ));

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
            Config::AttributeCollect(_)
        ));
    }

    #[test]
    fn openapi_exposes_the_collect_config_and_its_field_comments() {
        use utoipa::OpenApi;
        let api = serde_json::to_value(crate::api::ApiDoc::openapi()).unwrap();
        let schemas = &api["components"]["schemas"];
        for name in [
            "AttributeCollect",
            "AttributeCollectRule",
            "AttributeCollectInput",
            "AttributeCollectPivot",
            "SnmpObjectFormat",
            "RegexExtractFormat",
        ] {
            assert!(
                schemas[name]["description"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()),
                "{name}"
            );
            assert_eq!(schemas[name]["additionalProperties"], json!(false));
            for (field, property) in schemas[name]["properties"].as_object().unwrap() {
                // Optional referenced types keep their field comment on the
                // non-null branch of oneOf, not on the surrounding property.
                let description = property.get("description").or_else(|| {
                    property
                        .get("oneOf")?
                        .as_array()?
                        .iter()
                        .find_map(|variant| variant.get("description"))
                });
                assert!(
                    description
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty()),
                    "{name}.{field}"
                );
            }
        }
        assert_eq!(
            schemas["AttributeCollect"]["required"],
            json!(["collections"])
        );
        assert_eq!(
            schemas["AttributeCollectRule"]["required"],
            json!(["lookup_table", "table", "columns"])
        );
        // Every binding field is optional now: a bare `{}` is a pivoted binding.
        assert!(
            schemas["AttributeCollectInput"]
                .get("required")
                .is_none_or(|r| r.as_array().is_some_and(Vec::is_empty)),
            "{}",
            schemas["AttributeCollectInput"]
        );
        assert_eq!(
            schemas["AttributeCollectPivot"]["required"],
            json!(["array", "name", "value"])
        );
        assert_eq!(
            schemas["RegexExtractFormat"]["required"],
            json!(["pattern"])
        );
        assert_eq!(
            schemas["SnmpObjectFormat"]["properties"]["strip_instance"]["default"],
            json!(true)
        );
        assert_eq!(
            schemas["RegexExtractFormat"]["properties"]["group"]["default"],
            json!(1)
        );
        assert!(schemas.get("NameFormat").is_some());
        assert!(schemas.get("AttributeCollectCast").is_some());
        assert!(
            schemas["AttributeCollect"]["properties"]
                .get("overwrite")
                .is_none()
        );
    }
}
