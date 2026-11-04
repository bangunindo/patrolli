// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::config::lookup_table as cfg;
use anyhow::{Result, bail};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

/// Index of a materialized column within a [`Schema`].
pub type ColumnIdx = u32;
/// Index of a materialized row within a table.
pub type RowId = u32;

/// The materialized columns of a table.
///
/// For prefix-keyed tables the prefix column is *not* here: it is consumed by
/// the index and never stored per row, which is what lets many prefixes
/// collapse onto a single deduplicated row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Schema {
    names: Box<[SmolStr]>,
    types: Box<[cfg::ValueType]>,
    by_name: FxHashMap<SmolStr, ColumnIdx>,
}

impl Schema {
    pub fn new(columns: Vec<(SmolStr, cfg::ValueType)>) -> Result<Self> {
        if columns.len() > ColumnIdx::MAX as usize {
            bail!("table has too many columns ({})", columns.len());
        }
        let mut by_name = FxHashMap::with_capacity_and_hasher(columns.len(), Default::default());
        for (i, (name, _)) in columns.iter().enumerate() {
            if by_name.insert(name.clone(), i as ColumnIdx).is_some() {
                bail!("duplicate column {name:?}");
            }
        }
        let (names, types) = columns.into_iter().unzip::<_, _, Vec<_>, Vec<_>>();
        Ok(Self {
            names: names.into_boxed_slice(),
            types: types.into_boxed_slice(),
            by_name,
        })
    }

    pub fn index_of(&self, name: &str) -> Option<ColumnIdx> {
        self.by_name.get(name).copied()
    }

    pub fn name(&self, idx: ColumnIdx) -> Option<&str> {
        self.names.get(idx as usize).map(SmolStr::as_str)
    }

    pub fn value_type(&self, idx: ColumnIdx) -> Option<cfg::ValueType> {
        self.types.get(idx as usize).copied()
    }

    pub fn names(&self) -> &[SmolStr] {
        &self.names
    }

    pub fn types(&self) -> &[cfg::ValueType] {
        &self.types
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Resolves every name to its index, erroring on the first unknown one.
    pub fn resolve_all(&self, names: &[String], role: &str) -> Result<Box<[ColumnIdx]>> {
        names
            .iter()
            .map(|n| {
                self.index_of(n)
                    .ok_or_else(|| anyhow::anyhow!("{role} {n:?} is not a column of this table"))
            })
            .collect()
    }
}

/// The key configuration resolved against a concrete [`Schema`].
#[derive(Debug, Clone)]
pub enum KeyPlan {
    Hash {
        columns: Box<[ColumnIdx]>,
    },
    Prefix {
        /// indices into the materialized schema; empty for an unpartitioned table
        partition: Box<[ColumnIdx]>,
    },
}

impl KeyPlan {
    /// How many key parts a probe must supply.
    ///
    /// For a prefix key this counts only the partition columns; the address is
    /// passed separately.
    pub fn arity(&self) -> usize {
        match self {
            KeyPlan::Hash { columns } => columns.len(),
            KeyPlan::Prefix { partition } => partition.len(),
        }
    }

    pub fn is_prefix(&self) -> bool {
        matches!(self, KeyPlan::Prefix { .. })
    }
}

/// Column layout derived from the key config: which source columns are
/// materialized, and which are consumed by the index.
#[derive(Debug, Clone)]
pub struct ColumnLayout {
    /// source column index -> materialized index, or `None` when index-only
    pub materialized: Box<[Option<ColumnIdx>]>,
    /// source index of the column the index consumes (a prefix table's
    /// prefix column)
    pub key_column: Option<usize>,
}

impl ColumnLayout {
    /// Splits `source` (the columns as they appear in the file) into the
    /// materialized [`Schema`] and the index-only prefix column.
    pub fn build(
        source: &[(SmolStr, cfg::ValueType)],
        key: &cfg::Key,
    ) -> Result<(Self, Schema, KeyPlan)> {
        let position = |name: &str| source.iter().position(|(n, _)| n == name);

        let key_column = match key {
            cfg::Key::Hash(_) => None,
            cfg::Key::Prefix(prefix) => Some(position(&prefix.prefix_column).ok_or_else(|| {
                anyhow::anyhow!(
                    "key.prefix_column {:?} is not a column of this table",
                    prefix.prefix_column
                )
            })?),
        };

        let mut materialized = Vec::with_capacity(source.len());
        let mut columns = Vec::with_capacity(source.len());
        for (i, (name, ty)) in source.iter().enumerate() {
            if Some(i) == key_column {
                materialized.push(None);
                continue;
            }
            materialized.push(Some(columns.len() as ColumnIdx));
            columns.push((name.clone(), *ty));
        }
        if columns.is_empty() {
            bail!("table has no value columns once the prefix column is consumed");
        }
        let schema = Schema::new(columns)?;

        let plan = match key {
            cfg::Key::Hash(hash) => KeyPlan::Hash {
                columns: schema.resolve_all(&hash.key_columns, "key.key_columns entry")?,
            },
            cfg::Key::Prefix(prefix) => KeyPlan::Prefix {
                partition: schema
                    .resolve_all(&prefix.partition_columns, "key.partition_columns entry")?,
            },
        };

        Ok((
            Self {
                materialized: materialized.into_boxed_slice(),
                key_column,
            },
            schema,
            plan,
        ))
    }
}
