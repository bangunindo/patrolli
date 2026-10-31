// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;

use anyhow::bail;
use otel_arrow_dfe_pdata::otap::transform::{
    AttributesTransform, DeleteTransform, InsertTransform, RenameTransform, UpsertTransform,
};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use strum_macros::{Display, EnumDiscriminants, EnumString, IntoStaticStr};
use utoipa::ToSchema;
use uuid::Uuid;

pub mod attribute_collect;
pub mod attribute_lookup;
pub mod transform;
pub use attribute_collect::AttributeCollect;
pub use attribute_lookup::AttributeLookup;
pub use transform::Transform;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Processor {
    pub id: Uuid,
    pub name: String,
    #[serde(flatten)]
    pub config: Config,
    #[serde(default)]
    pub inputs: Vec<Uuid>,
}

impl Processor {
    pub fn validate(&mut self) -> anyhow::Result<()> {
        self.config.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, EnumDiscriminants, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type", content = "config")]
#[schema(as = ProcessorConfig)]
#[strum_discriminants(
    name(ProcessorKind),
    derive(Serialize, Deserialize, EnumString, Display, ToSchema, IntoStaticStr),
    vis(pub),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
pub enum Config {
    // Null transform is useful for routing metrics
    // TODO: add routing processor
    Null,
    AttributeFilter(AttributeFilter),
    DropAttributes(DropAttributes),
    AddAttributes(AddAttributes),
    RenameAttributes(RenameAttributes),
    AttributeLookup(AttributeLookup),
    AttributeCollect(AttributeCollect),
    Transform(Transform),
}

impl Config {
    pub fn validate(&mut self) -> anyhow::Result<()> {
        match self {
            Config::Null => {}
            Config::AttributeFilter(filter) => filter.validate()?,
            Config::DropAttributes(processor) => processor.validate()?,
            Config::AddAttributes(processor) => processor.validate()?,
            Config::RenameAttributes(processor) => processor.validate()?,
            Config::AttributeLookup(processor) => processor.validate()?,
            Config::AttributeCollect(processor) => processor.validate()?,
            Config::Transform(processor) => processor.validate()?,
        }
        Ok(())
    }

    pub fn to_json_value(&self) -> anyhow::Result<serde_json::Value> {
        match self {
            Config::Null => Ok(serde_json::Value::Null),
            Config::AttributeFilter(config) => Ok(serde_json::to_value(config)?),
            Config::DropAttributes(config) => Ok(serde_json::to_value(config)?),
            Config::AddAttributes(config) => Ok(serde_json::to_value(config)?),
            Config::RenameAttributes(config) => Ok(serde_json::to_value(config)?),
            Config::AttributeLookup(config) => Ok(serde_json::to_value(config)?),
            Config::AttributeCollect(config) => Ok(serde_json::to_value(config)?),
            Config::Transform(config) => Ok(serde_json::to_value(config)?),
        }
    }
}

// Type alias because utoipa doesn't work with types literally named "Value"
pub type FilterValue = Value;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = FilterValue)]
pub enum Value {
    Integer(i64),
    Float(f64),
    #[schema(value_type = String)]
    String(SmolStr),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum KeyValueOp {
    #[serde(rename = "=")]
    Equal,
    #[serde(rename = "=~")]
    RegexEqual,
    #[serde(rename = "!=")]
    NotEqual,
    #[serde(rename = "!~")]
    RegexNotEqual,
    #[serde(rename = "<")]
    LessThan,
    #[serde(rename = "<=")]
    LessThanOrEqual,
    #[serde(rename = ">")]
    GreaterThan,
    #[serde(rename = ">=")]
    GreaterThanOrEqual,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AttributeTable {
    Record,
    Scope,
    Resource,
}

impl AttributeTable {
    pub fn as_str(&self) -> &'static str {
        match self {
            // record_attrs is an alias for log_attrs/span_attrs/metric_attrs. depends on the type of data
            AttributeTable::Record => "record_attrs",
            AttributeTable::Scope => "scope_attrs",
            AttributeTable::Resource => "resource_attrs",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributeFilterExpr {
    pub table: AttributeTable,
    pub key: String,
    pub key_is_regex: bool,
    pub op: KeyValueOp,
    #[schema(value_type = FilterValue)]
    pub value: Value,
}

impl AttributeFilterExpr {
    pub fn validate(&self) -> anyhow::Result<()> {
        if matches!(self.value, Value::Integer(_) | Value::Float(_))
            && matches!(self.op, KeyValueOp::RegexEqual | KeyValueOp::RegexNotEqual)
        {
            bail!("Regex operations are not supported for integer or float values");
        }
        Ok(())
    }
}

// TODO: array filter. let's say we have [{oid: 1.1.1, val: 2}, {oid: 1.1.2, val: 3}]
//  and we want to match the value with oid 1.1.1
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttributeFilter {
    #[schema(no_recursion)]
    And(Vec<AttributeFilter>),
    #[schema(no_recursion)]
    Or(Vec<AttributeFilter>),
    Match(AttributeFilterExpr),
}

impl AttributeFilter {
    pub fn validate(&mut self) -> anyhow::Result<()> {
        match self {
            AttributeFilter::And(filters) | AttributeFilter::Or(filters) => {
                for f in filters {
                    f.validate()?;
                }
            }
            AttributeFilter::Match(entry) => entry.validate()?,
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct TableTransform {
    pub record: Option<AttributesTransform>,
    pub scope: Option<AttributesTransform>,
    pub resource: Option<AttributesTransform>,
}

impl TableTransform {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let Some(record_transform) = &self.record {
            record_transform.validate()?;
        }
        if let Some(scope_transform) = &self.scope {
            scope_transform.validate()?;
        }
        if let Some(resource_transform) = &self.resource {
            resource_transform.validate()?;
        }
        Ok(())
    }
}

#[derive(derive_more::Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DropAttributes {
    pub keys: HashMap<AttributeTable, Vec<String>>,
    #[serde(skip)]
    #[debug(skip)]
    pub(crate) transforms: Option<TableTransform>,
}

impl Clone for DropAttributes {
    fn clone(&self) -> Self {
        let mut s = Self {
            keys: self.keys.clone(),
            transforms: None,
        };

        // Populate transforms only if they exist, so that the clone has the same transforms as the original
        if self.transforms.is_some() {
            s.populate_transforms();
        }

        s
    }
}

impl DropAttributes {
    pub fn populate_transforms(&mut self) {
        let mut transforms = TableTransform::default();
        for (table, keys) in &self.keys {
            if keys.is_empty() {
                continue;
            }

            let transform = AttributesTransform::default()
                .with_delete(DeleteTransform::new(keys.iter().cloned().collect()));
            match table {
                AttributeTable::Record => transforms.record = Some(transform),
                AttributeTable::Scope => transforms.scope = Some(transform),
                AttributeTable::Resource => transforms.resource = Some(transform),
            }
        }

        self.transforms = Some(transforms);
    }

    pub fn validate(&mut self) -> anyhow::Result<()> {
        self.populate_transforms();

        if let Some(transforms) = &self.transforms {
            transforms.validate()?;
        }

        Ok(())
    }
}

#[derive(derive_more::Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AddAttributes {
    pub kv_map: HashMap<AttributeTable, BTreeMap<String, super::OtelValue>>,
    pub overwrite: bool,
    #[serde(skip)]
    #[debug(skip)]
    pub(crate) transforms: Option<TableTransform>,
}

impl Clone for AddAttributes {
    fn clone(&self) -> Self {
        let mut s = Self {
            kv_map: self.kv_map.clone(),
            overwrite: self.overwrite,
            transforms: None,
        };

        if self.transforms.is_some() {
            s.populate_transforms();
        }

        s
    }
}

impl AddAttributes {
    pub fn populate_transforms(&mut self) {
        let mut transforms = TableTransform::default();
        for (table, kv_map) in &self.kv_map {
            if kv_map.is_empty() {
                continue;
            }

            let transform = if self.overwrite {
                AttributesTransform::default().with_upsert(UpsertTransform::new(
                    kv_map
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone().into()))
                        .collect(),
                ))
            } else {
                AttributesTransform::default().with_insert(InsertTransform::new(
                    kv_map
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone().into()))
                        .collect(),
                ))
            };

            match table {
                AttributeTable::Record => transforms.record = Some(transform),
                AttributeTable::Scope => transforms.scope = Some(transform),
                AttributeTable::Resource => transforms.resource = Some(transform),
            }
        }

        self.transforms = Some(transforms);
    }

    pub fn validate(&mut self) -> anyhow::Result<()> {
        self.populate_transforms();

        if let Some(transforms) = &self.transforms {
            transforms.validate()?;
        }

        Ok(())
    }
}

#[derive(derive_more::Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RenameAttributes {
    pub rename_map: HashMap<AttributeTable, BTreeMap<String, String>>,
    #[serde(skip)]
    #[debug(skip)]
    pub(crate) transforms: Option<TableTransform>,
}

impl Clone for RenameAttributes {
    fn clone(&self) -> Self {
        let mut s = Self {
            rename_map: self.rename_map.clone(),
            transforms: None,
        };

        if self.transforms.is_some() {
            s.populate_transforms();
        }

        s
    }
}

impl RenameAttributes {
    pub fn populate_transforms(&mut self) {
        let mut transforms = TableTransform::default();
        for (table, rename_map) in &self.rename_map {
            if rename_map.is_empty() {
                continue;
            }

            let transform = AttributesTransform::default()
                .with_rename(RenameTransform::new(rename_map.clone()));
            match table {
                AttributeTable::Record => transforms.record = Some(transform),
                AttributeTable::Scope => transforms.scope = Some(transform),
                AttributeTable::Resource => transforms.resource = Some(transform),
            }
        }

        self.transforms = Some(transforms);
    }

    pub fn validate(&mut self) -> anyhow::Result<()> {
        self.populate_transforms();

        if let Some(transforms) = &self.transforms {
            transforms.validate()?;
        }

        Ok(())
    }
}
