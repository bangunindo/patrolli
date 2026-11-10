// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::cell::{CellRef, KeyRef, Lookup, hash_key};
use super::column::{Column, ColumnBuilder, Interner, StagedCell};
use super::index::{HashIndex, Index, PrefixIndexBuilder, PrefixReject};
use super::prefix::parse_prefix;
use super::schema::{ColumnIdx, ColumnLayout, KeyPlan, RowId, Schema};
use crate::config::lookup_table as cfg;
use anyhow::{Result, bail};
use hashbrown::HashTable;
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use utoipa::ToSchema;
use xxhash_rust::xxh3::Xxh3;

/// Why a source row was not stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowReject {
    /// the row has a different number of fields than the schema
    Arity,
    /// a cell could not be cast to its column type
    BadCell,
    /// the prefix column was null, not a CIDR, or had host bits set
    BadPrefix,
    /// a key or partition column was null, which no probe can name
    NullKey,
    /// another row already claims this exact key (for a prefix table, this
    /// exact prefix in this partition)
    DuplicateKey,
}

impl RowReject {
    pub fn as_str(self) -> &'static str {
        match self {
            RowReject::Arity => "field count",
            RowReject::BadCell => "cast",
            RowReject::BadPrefix => "prefix",
            RowReject::NullKey => "null key",
            RowReject::DuplicateKey => "duplicate key",
        }
    }
}

impl From<PrefixReject> for RowReject {
    fn from(r: PrefixReject) -> Self {
        match r {
            PrefixReject::Duplicate => RowReject::DuplicateKey,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowOutcome {
    /// stored as a new row
    Inserted,
    /// an identical row already existed; only the index gained an entry
    Deduplicated,
    Rejected(RowReject),
}

/// Per-reason skip counts, so the warning at the end of a load says what was
/// wrong rather than just how much.
///
/// The serialized names are shared with [`SkipCounts::summary`] and
/// `internal_metrics::visit::SKIP_REASONS`, so a log line, a Prometheus
/// `reason` label and a JSON key name the same rejection. Renaming a field
/// here renames it in all three.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SkipCounts {
    #[serde(rename = "field_count")]
    pub arity: u64,
    #[serde(rename = "cast")]
    pub bad_cell: u64,
    #[serde(rename = "prefix")]
    pub bad_prefix: u64,
    pub null_key: u64,
    pub duplicate_key: u64,
}

impl SkipCounts {
    pub fn record(&mut self, reject: RowReject) {
        let slot = match reject {
            RowReject::Arity => &mut self.arity,
            RowReject::BadCell => &mut self.bad_cell,
            RowReject::BadPrefix => &mut self.bad_prefix,
            RowReject::NullKey => &mut self.null_key,
            RowReject::DuplicateKey => &mut self.duplicate_key,
        };
        *slot += 1;
    }

    pub fn total(&self) -> u64 {
        self.arity + self.bad_cell + self.bad_prefix + self.null_key + self.duplicate_key
    }

    /// A compact `reason=count` summary of the non-zero reasons.
    pub fn summary(&self) -> String {
        let parts = [
            ("field_count", self.arity),
            ("cast", self.bad_cell),
            ("prefix", self.bad_prefix),
            ("null_key", self.null_key),
            ("duplicate_key", self.duplicate_key),
        ];
        parts
            .iter()
            .filter(|(_, n)| *n > 0)
            .map(|(name, n)| format!("{name}={n}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// How one generation of an immutable table was built.
///
/// `PartialEq`/`Eq` are derived because the status API nests this and derives
/// `PartialEq` for its round-trip test.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LoadStats {
    pub rows_read: u64,
    pub skipped: SkipCounts,
    /// Rows stored after identical ones collapsed. Equal to the table's row
    /// count by construction, as is `index_entries`.
    pub unique_rows: u64,
    pub index_entries: u64,
    /// Millis on the wire, matching the `elapsed_ms` the loader logs.
    #[serde(with = "crate::common::serde::duration_millis")]
    #[schema(value_type = u64)]
    pub elapsed: Duration,
    /// the first rejection message, to make a warning actionable
    pub first_error: Option<String>,
}

/// One immutable generation of a table: columns, interner and index, all built
/// once and then read-only.
#[derive(Debug)]
pub struct ImmutableData {
    schema: Arc<Schema>,
    key: KeyPlan,
    rows: u32,
    columns: Box<[Column]>,
    strings: Interner,
    index: Index,
    stats: LoadStats,
    generation: u64,
}

impl ImmutableData {
    /// The state a table is in before its first successful load.
    pub fn empty(schema: Arc<Schema>, key: KeyPlan) -> Self {
        let columns = schema
            .types()
            .iter()
            .map(|ty| ColumnBuilder::new(*ty).finish())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let index = match &key {
            KeyPlan::Hash { .. } => Index::Hash(HashIndex::default()),
            KeyPlan::Prefix { partition } => {
                Index::Prefix(PrefixIndexBuilder::new(!partition.is_empty()).finish())
            }
        };
        Self {
            schema,
            key,
            rows: 0,
            columns,
            strings: Interner::new(),
            index,
            stats: LoadStats::default(),
            generation: 0,
        }
    }

    /// The placeholder a file-backed table holds before its first successful
    /// load: no columns, and an index that matches nothing.
    pub fn not_loaded(key: &cfg::Key) -> Self {
        let plan = match key {
            cfg::Key::Hash(_) => KeyPlan::Hash {
                columns: Box::new([]),
            },
            cfg::Key::Prefix(_) => KeyPlan::Prefix {
                partition: Box::new([]),
            },
        };
        Self::empty(Arc::new(Schema::default()), plan)
    }

    pub fn schema(&self) -> &Arc<Schema> {
        &self.schema
    }

    pub fn key_plan(&self) -> &KeyPlan {
        &self.key
    }

    pub fn rows(&self) -> usize {
        self.rows as usize
    }

    pub fn index_entries(&self) -> usize {
        self.index.len()
    }

    /// Distinct strings held by this generation's interner: every value of
    /// every string column, interned once, rebuilt per generation.
    ///
    /// Counts a different population from
    /// [`super::mutable::Mutable::pooled_strings`], which pools only strings
    /// too long to live inline in a `SmolStr`.
    pub fn interned_strings(&self) -> usize {
        self.strings.len()
    }

    pub fn stats(&self) -> &LoadStats {
        &self.stats
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    #[inline]
    pub fn cell(&self, row: RowId, column: ColumnIdx) -> CellRef<'_> {
        match self.columns.get(column as usize) {
            Some(c) => c.get(row as usize, &self.strings),
            None => CellRef::Null,
        }
    }

    /// The matching row, and for a prefix table the prefix that matched.
    pub fn lookup(&self, key: Lookup<'_>) -> Option<(RowId, Option<IpNet>)> {
        match (&self.index, key) {
            (Index::Hash(index), Lookup::Hash(parts)) => {
                let KeyPlan::Hash { columns } = &self.key else {
                    return None;
                };
                if parts.len() != columns.len() {
                    return None;
                }
                let hash = hash_key(parts.iter().copied());
                index
                    .find(hash, |row| {
                        columns
                            .iter()
                            .zip(parts)
                            .all(|(col, part)| cell_matches_key(self.cell(row, *col), *part))
                    })
                    .map(|row| (row, None))
            }
            (Index::Prefix(index), Lookup::Prefix { partition, addr }) => {
                let KeyPlan::Prefix { partition: cols } = &self.key else {
                    return None;
                };
                if partition.len() != cols.len() {
                    return None;
                }
                index
                    .get(partition, addr)
                    .map(|(row, net)| (row, Some(net)))
            }
            // a hash probe against a prefix table, or the reverse
            _ => None,
        }
    }
}

#[inline]
fn cell_matches_key(cell: CellRef<'_>, key: KeyRef<'_>) -> bool {
    match (cell, key) {
        (CellRef::Bool(a), KeyRef::Bool(b)) => a == b,
        (CellRef::Integer(a), KeyRef::Integer(b)) => a == b,
        (CellRef::String(a), KeyRef::String(b)) => a == b,
        (CellRef::Ip(a), KeyRef::Ip(b)) => a == b,
        _ => false,
    }
}

/// Resolves the source columns, forcing the prefix column to `String`
/// regardless of any configured cast: it is CIDR text parsed by the index, not
/// a value.
pub fn source_columns(
    names: &[SmolStr],
    key: &cfg::Key,
    type_of: impl Fn(&str) -> cfg::ValueType,
) -> Vec<(SmolStr, cfg::ValueType)> {
    let prefix_column = match key {
        cfg::Key::Prefix(prefix) => Some(prefix.prefix_column.as_str()),
        cfg::Key::Hash(_) => None,
    };
    names
        .iter()
        .map(|name| {
            let ty = if Some(name.as_str()) == prefix_column {
                cfg::ValueType::String
            } else {
                type_of(name)
            };
            (name.clone(), ty)
        })
        .collect()
}

/// One row as it arrives from a source.
///
/// Cells are pulled per column and parsed on demand, so a text source never
/// has to materialize an intermediate buffer of borrowed cells just to hand it
/// to the builder.
pub trait RowSource {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn cell(&self, index: usize, ty: cfg::ValueType) -> Result<CellRef<'_>>;
}

/// A row that is already parsed, for non-text sources and tests.
pub struct Cells<'a>(pub &'a [CellRef<'a>]);

impl RowSource for Cells<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn cell(&self, index: usize, _ty: cfg::ValueType) -> Result<CellRef<'_>> {
        self.0
            .get(index)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("no field at index {index}"))
    }
}

pub struct ImmutableBuilder {
    rows: RowsBuilder,
    state: BuilderState,
}

/// Parsing and materialized storage shared by both key kinds.
struct RowsBuilder {
    schema: Arc<Schema>,
    materialized: Box<[Option<ColumnIdx>]>,
    source_types: Box<[cfg::ValueType]>,
    columns: Vec<ColumnBuilder>,
    strings: Interner,
    len: u32,
    stats: LoadStats,
}

/// Each variant owns its resolved key and the state needed to build its index.
enum BuilderState {
    Hash(HashBuilder),
    Prefix(PrefixBuilder),
}

struct HashBuilder {
    key_columns: Box<[ColumnIdx]>,
    index: HashIndex,
}

struct PrefixBuilder {
    partition: Box<[ColumnIdx]>,
    /// source index of the prefix column
    prefix_column: usize,
    index: PrefixIndexBuilder,
    /// row fingerprint -> row id; only prefix tables can share materialized rows
    dedup: HashTable<RowId>,
}

impl ImmutableBuilder {
    /// `source` is the column list exactly as it appears in the file, already
    /// typed by [`source_columns`].
    pub fn new(source: &[(SmolStr, cfg::ValueType)], key: &cfg::Key) -> Result<Self> {
        if source.is_empty() {
            bail!("table has no columns");
        }
        let (layout, schema, plan) = ColumnLayout::build(source, key)?;
        let state = match (plan, layout.key_column) {
            (KeyPlan::Hash { columns }, None) => BuilderState::Hash(HashBuilder {
                key_columns: columns,
                index: HashIndex::default(),
            }),
            (KeyPlan::Prefix { partition }, Some(prefix_column)) => {
                BuilderState::Prefix(PrefixBuilder {
                    index: PrefixIndexBuilder::new(!partition.is_empty()),
                    partition,
                    prefix_column,
                    dedup: HashTable::new(),
                })
            }
            _ => bail!("inconsistent lookup table key and column layout"),
        };
        let columns = schema
            .types()
            .iter()
            .map(|t| ColumnBuilder::new(*t))
            .collect();
        Ok(Self {
            rows: RowsBuilder {
                schema: Arc::new(schema),
                materialized: layout.materialized,
                source_types: source.iter().map(|(_, t)| *t).collect(),
                columns,
                strings: Interner::new(),
                len: 0,
                stats: LoadStats::default(),
            },
            state,
        })
    }

    /// Adds one source row. The row must have one field per source column,
    /// including the prefix column.
    pub fn push_row(&mut self, source: &impl RowSource) -> RowOutcome {
        let rows = &mut self.rows;
        rows.stats.rows_read += 1;
        let row_no = rows.stats.rows_read;

        if source.len() != rows.source_types.len() {
            let (got, want) = (source.len(), rows.source_types.len());
            return rows.reject(RowReject::Arity, || {
                format!("row {row_no} has {got} fields, expected {want}")
            });
        }

        match &mut self.state {
            BuilderState::Hash(hash) => hash.push_row(rows, source),
            BuilderState::Prefix(prefix) => prefix.push_row(rows, source),
        }
    }

    pub fn finish(self, elapsed: Duration) -> Result<ImmutableData> {
        let RowsBuilder {
            schema,
            columns,
            mut strings,
            len,
            mut stats,
            ..
        } = self.rows;
        let skipped = stats.skipped.total();
        if len == 0 && skipped > 0 {
            bail!(
                "every row was rejected ({skipped} row(s): {}){}",
                stats.skipped.summary(),
                stats
                    .first_error
                    .as_deref()
                    .map(|e| format!("; first: {e}"))
                    .unwrap_or_default()
            );
        }
        strings.shrink_to_fit();
        let columns: Box<[Column]> = columns
            .into_iter()
            .map(ColumnBuilder::finish)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let (key, index) = self.state.finish();
        stats.unique_rows = len as u64;
        stats.index_entries = index.len() as u64;
        stats.elapsed = elapsed;
        Ok(ImmutableData {
            schema,
            key,
            rows: len,
            columns,
            strings,
            index,
            stats,
            generation: 0,
        })
    }
}

impl RowsBuilder {
    /// Parses without committing strings. Borrowed cells live only for this row.
    fn stage<'s>(
        &mut self,
        source: &'s impl RowSource,
    ) -> Result<SmallVec<[CellRef<'s>; 16]>, RowOutcome> {
        let mut staged = SmallVec::with_capacity(self.columns.len());
        for i in 0..self.source_types.len() {
            if self.materialized[i].is_none() {
                continue;
            }
            match source.cell(i, self.source_types[i]) {
                Ok(cell) => staged.push(cell),
                Err(e) => {
                    let row_no = self.stats.rows_read;
                    return Err(self.reject(RowReject::BadCell, || {
                        format!("row {row_no} column {i}: {e}")
                    }));
                }
            }
        }
        Ok(staged)
    }

    fn reject(&mut self, reject: RowReject, detail: impl FnOnce() -> String) -> RowOutcome {
        self.stats.skipped.record(reject);
        if self.stats.first_error.is_none() {
            self.stats.first_error = Some(detail());
        }
        RowOutcome::Rejected(reject)
    }

    fn materialize(&mut self, staged: &[CellRef<'_>]) {
        for (builder, cell) in self.columns.iter_mut().zip(staged) {
            builder
                .push(StagedCell::stage(*cell, &mut self.strings))
                .expect("staged cells are built from the column's own type");
        }
        self.len += 1;
    }
}

impl BuilderState {
    fn finish(self) -> (KeyPlan, Index) {
        match self {
            BuilderState::Hash(hash) => (
                KeyPlan::Hash {
                    columns: hash.key_columns,
                },
                Index::Hash(hash.index),
            ),
            BuilderState::Prefix(prefix) => (
                KeyPlan::Prefix {
                    partition: prefix.partition,
                },
                Index::Prefix(prefix.index.finish()),
            ),
        }
    }
}

impl HashBuilder {
    fn push_row(&mut self, rows: &mut RowsBuilder, source: &impl RowSource) -> RowOutcome {
        let staged = match rows.stage(source) {
            Ok(staged) => staged,
            Err(outcome) => return outcome,
        };
        let row_no = rows.stats.rows_read;
        let key_cols = &self.key_columns;
        let hash = match staged_key_hash(&staged, key_cols) {
            Ok(hash) => hash,
            Err(reject) => {
                return rows.reject(reject, || format!("row {row_no} {}", reject.as_str()));
            }
        };
        let accepted = self.index.insert_unique(
            hash,
            rows.len,
            |other| {
                key_cols.iter().all(|c| {
                    cells_equal(
                        stored_cell(rows.columns[*c as usize].get(other as usize), &rows.strings),
                        staged[*c as usize],
                    )
                })
            },
            |other| row_key_hash(other, key_cols, &rows.columns, &rows.strings).unwrap_or_default(),
        );
        if !accepted {
            return rows.reject(RowReject::DuplicateKey, || {
                format!("row {row_no} {}", RowReject::DuplicateKey.as_str())
            });
        }
        rows.materialize(&staged);
        RowOutcome::Inserted
    }
}

impl PrefixBuilder {
    fn push_row(&mut self, rows: &mut RowsBuilder, source: &impl RowSource) -> RowOutcome {
        let row_no = rows.stats.rows_read;
        // Parse the prefix before materialized cells so an unusable one is
        // cheap to reject.
        let net = match source.cell(self.prefix_column, cfg::ValueType::String) {
            Ok(CellRef::String(raw)) => parse_prefix(raw).map_err(|e| e.to_string()),
            Ok(CellRef::Null) => Err("is null".to_owned()),
            Ok(_) => Err("is not text".to_owned()),
            Err(e) => Err(e.to_string()),
        };
        let net = match net {
            Ok(net) => net,
            Err(e) => {
                return rows.reject(RowReject::BadPrefix, || format!("row {row_no} prefix: {e}"));
            }
        };
        let staged = match rows.stage(source) {
            Ok(staged) => staged,
            Err(outcome) => return outcome,
        };

        // Only prefixes can share a materialized row while naming distinct keys.
        let hash = fingerprint(staged.iter().copied());
        let existing = self
            .dedup
            .find(hash, |row| {
                rows.columns.iter().enumerate().all(|(i, c)| {
                    cells_equal(stored_cell(c.get(*row as usize), &rows.strings), staged[i])
                })
            })
            .copied();
        let row_id = existing.unwrap_or(rows.len);

        let mut parts: SmallVec<[KeyRef<'_>; 8]> = SmallVec::with_capacity(self.partition.len());
        for col in &self.partition {
            match cell_key(staged[*col as usize]) {
                Ok(key) => parts.push(key),
                Err(reject) => {
                    return rows.reject(reject, || format!("row {row_no} {}", reject.as_str()));
                }
            }
        }
        if let Err(reject) = self.index.insert(&parts, net, row_id) {
            let reject = RowReject::from(reject);
            return rows.reject(reject, || format!("row {row_no} {}", reject.as_str()));
        }

        if existing.is_some() {
            return RowOutcome::Deduplicated;
        }
        rows.materialize(&staged);
        self.dedup.insert_unique(hash, row_id, |other| {
            fingerprint(
                rows.columns
                    .iter()
                    .map(|column| stored_cell(column.get(*other as usize), &rows.strings)),
            )
        });
        RowOutcome::Inserted
    }
}

fn fingerprint<'a>(cells: impl IntoIterator<Item = CellRef<'a>>) -> u64 {
    let mut h = Xxh3::new();
    for cell in cells {
        cell_hash(cell, &mut h);
    }
    h.digest()
}

fn cell_hash(cell: CellRef<'_>, h: &mut Xxh3) {
    match cell {
        CellRef::Null => h.update(&[0]),
        CellRef::Bool(v) => h.update(&[1, v as u8]),
        CellRef::Double(v) => {
            h.update(&[2]);
            h.update(&v.to_bits().to_le_bytes());
        }
        CellRef::Integer(v) => {
            h.update(&[3]);
            h.update(&v.to_le_bytes());
        }
        CellRef::String(v) => {
            h.update(&[4]);
            h.update(&(v.len() as u64).to_le_bytes());
            h.update(v.as_bytes());
        }
        CellRef::Ip(IpAddr::V4(v)) => {
            h.update(&[5]);
            h.update(&v.octets());
        }
        CellRef::Ip(IpAddr::V6(v)) => {
            h.update(&[6]);
            h.update(&v.octets());
        }
    }
}

fn stored_cell(cell: StagedCell, strings: &Interner) -> CellRef<'_> {
    match cell {
        StagedCell::Null => CellRef::Null,
        StagedCell::Bool(v) => CellRef::Bool(v),
        StagedCell::Integer(v) => CellRef::Integer(v),
        StagedCell::Double(v) => CellRef::Double(f64::from_bits(v)),
        StagedCell::String(sym) => CellRef::String(strings.resolve(sym).expect("stored symbol")),
        StagedCell::Ip(v) => CellRef::Ip(v),
    }
}

fn cells_equal(a: CellRef<'_>, b: CellRef<'_>) -> bool {
    match (a, b) {
        (CellRef::Double(a), CellRef::Double(b)) => a.to_bits() == b.to_bits(),
        _ => a == b,
    }
}

/// Reports *why* a cell cannot be a key, so the two reasons land in different
/// skip counters instead of both reading as a failed cast.
fn cell_key(cell: CellRef<'_>) -> Result<KeyRef<'_>, RowReject> {
    Ok(match cell {
        CellRef::Bool(v) => KeyRef::Bool(v),
        CellRef::Integer(v) => KeyRef::Integer(v),
        CellRef::String(v) => KeyRef::String(v),
        CellRef::Ip(v) => KeyRef::Ip(v),
        // No probe can name a null key — see `KeyRef` — so the row would be
        // unreachable, and the next row with the same blank key would then be
        // miscounted as a duplicate of it.
        CellRef::Null => return Err(RowReject::NullKey),
        // doubles are rejected as keys during config validation, but a
        // mis-cast column could still land one here
        CellRef::Double(_) => return Err(RowReject::BadCell),
    })
}

/// Hashes a key straight out of the staged row. Must agree with the probe path
/// in [`ImmutableData::lookup`], so both go through `cell::hash_key`.
fn staged_key_hash(staged: &[CellRef<'_>], columns: &[ColumnIdx]) -> Result<u64, RowReject> {
    let mut keys: SmallVec<[KeyRef<'_>; 8]> = SmallVec::with_capacity(columns.len());
    for col in columns {
        keys.push(cell_key(staged[*col as usize])?);
    }
    Ok(hash_key(keys.iter().copied()))
}

/// The same hash, read back out of an already-stored row.
fn row_key_hash(
    row: RowId,
    columns: &[ColumnIdx],
    builders: &[ColumnBuilder],
    strings: &Interner,
) -> Option<u64> {
    let mut keys: SmallVec<[KeyRef<'_>; 8]> = SmallVec::with_capacity(columns.len());
    for col in columns {
        keys.push(
            cell_key(stored_cell(
                builders[*col as usize].get(row as usize),
                strings,
            ))
            .ok()?,
        );
    }
    Some(hash_key(keys.iter().copied()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::lookup_table::{Hash as HashKey, Prefix as PrefixKey, ValueType};

    fn hash_key_cfg(columns: &[&str]) -> cfg::Key {
        cfg::Key::Hash(HashKey {
            key_columns: columns.iter().map(|s| s.to_string()).collect(),
        })
    }

    #[test]
    fn rejected_rows_never_intern_strings() {
        let key = hash_key_cfg(&["host"]);
        let mut builder = ImmutableBuilder::new(
            &source(
                &[("host", ValueType::String), ("site", ValueType::String)],
                &key,
            ),
            &key,
        )
        .unwrap();
        builder.push_row(&Cells(&[CellRef::String("a"), CellRef::String("kept")]));
        for i in 0..10_000 {
            let text = format!("unique rejected value {i}");
            assert_eq!(
                builder.push_row(&Cells(&[CellRef::String("a"), CellRef::String(&text)])),
                RowOutcome::Rejected(RowReject::DuplicateKey)
            );
        }
        struct LateError;
        impl RowSource for LateError {
            fn len(&self) -> usize {
                2
            }
            fn cell(&self, index: usize, _: ValueType) -> Result<CellRef<'_>> {
                if index == 0 {
                    Ok(CellRef::String("must not be interned"))
                } else {
                    bail!("late cast failure")
                }
            }
        }
        assert_eq!(
            builder.push_row(&LateError),
            RowOutcome::Rejected(RowReject::BadCell)
        );
        let data = builder.finish(Duration::ZERO).unwrap();
        assert_eq!(data.rows(), 1);
        assert_eq!(data.interned_strings(), 2);
        assert_eq!(data.stats().skipped.total(), 10_001);
    }

    #[test]
    fn rejected_prefixes_retain_neither_strings_nor_empty_partitions() {
        let key = prefix_key_cfg(&["host"]);
        let columns = source(
            &[
                ("host", ValueType::String),
                ("network", ValueType::String),
                ("site", ValueType::String),
            ],
            &key,
        );
        let mut builder = ImmutableBuilder::new(&columns, &key).unwrap();
        builder.push_row(&Cells(&[
            CellRef::String("a"),
            CellRef::String("10.0.0.0/8"),
            CellRef::String("kept"),
        ]));
        for i in 0..1000 {
            let text = format!("rejected-{i}");
            assert_eq!(
                builder.push_row(&Cells(&[
                    CellRef::String("a"),
                    CellRef::String("10.0.0.0/8"),
                    CellRef::String(&text)
                ])),
                RowOutcome::Rejected(RowReject::DuplicateKey)
            );
            assert_eq!(
                builder.push_row(&Cells(&[
                    CellRef::String(&text),
                    CellRef::String("10.0.0.1/8"),
                    CellRef::String(&text)
                ])),
                RowOutcome::Rejected(RowReject::BadPrefix)
            );
        }
        let data = builder.finish(Duration::ZERO).unwrap();
        assert_eq!(data.interned_strings(), 2);
        let Index::Prefix(index) = &data.index else {
            unreachable!()
        };
        assert_eq!(index.partitions(), 1);
    }

    fn prefix_key_cfg(partition: &[&str]) -> cfg::Key {
        cfg::Key::Prefix(PrefixKey {
            prefix_column: "network".into(),
            partition_columns: partition.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn at_prefix<'d>(
        data: &'d ImmutableData,
        partition: &[KeyRef<'_>],
        addr: &str,
        column: &str,
    ) -> Option<(CellRef<'d>, IpNet)> {
        let col = data.schema().index_of(column).unwrap();
        data.lookup(Lookup::Prefix {
            partition,
            addr: addr.parse().unwrap(),
        })
        .map(|(row, net)| (data.cell(row, col), net.expect("a prefix match")))
    }

    fn net(s: &str) -> IpNet {
        s.parse().unwrap()
    }

    /// Resolves source columns the way the loader does, so the prefix column
    /// override is exercised too.
    fn source(names: &[(&str, ValueType)], key: &cfg::Key) -> Vec<(SmolStr, ValueType)> {
        let casts: Vec<(SmolStr, ValueType)> =
            names.iter().map(|(n, t)| (SmolStr::new(*n), *t)).collect();
        let just_names: Vec<SmolStr> = casts.iter().map(|(n, _)| n.clone()).collect();
        source_columns(&just_names, key, |name| {
            casts
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| *t)
                .unwrap_or_default()
        })
    }

    fn build(
        names: &[(&str, ValueType)],
        key: &cfg::Key,
        rows: &[Vec<CellRef<'_>>],
    ) -> Result<ImmutableData> {
        let src = source(names, key);
        let mut b = ImmutableBuilder::new(&src, key)?;
        for row in rows {
            b.push_row(&Cells(row));
        }
        b.finish(Duration::ZERO)
    }

    #[test]
    fn hash_lookup_finds_composite_keys_and_misses_cleanly() {
        let key = hash_key_cfg(&["tenant", "code"]);
        let names = [
            ("tenant", ValueType::String),
            ("code", ValueType::Integer),
            ("city", ValueType::String),
        ];
        let data = build(
            &names,
            &key,
            &[
                vec![
                    CellRef::String("a"),
                    CellRef::Integer(1),
                    CellRef::String("jakarta"),
                ],
                vec![
                    CellRef::String("a"),
                    CellRef::Integer(2),
                    CellRef::String("bandung"),
                ],
                vec![
                    CellRef::String("b"),
                    CellRef::Integer(1),
                    CellRef::String("medan"),
                ],
            ],
        )
        .unwrap();

        let city = data.schema().index_of("city").unwrap();
        let probe = |t, c| {
            data.lookup(Lookup::Hash(&[KeyRef::String(t), KeyRef::Integer(c)]))
                .map(|(row, net)| {
                    assert_eq!(net, None, "a hash match has no prefix");
                    data.cell(row, city)
                })
        };
        assert_eq!(probe("a", 1), Some(CellRef::String("jakarta")));
        assert_eq!(probe("a", 2), Some(CellRef::String("bandung")));
        assert_eq!(probe("b", 1), Some(CellRef::String("medan")));
        assert_eq!(probe("b", 2), None, "combination is absent");
        assert_eq!(probe("z", 1), None);
        // an arity mismatch is a miss, not a panic
        assert_eq!(data.lookup(Lookup::Hash(&[KeyRef::String("a")])), None);
    }

    /// A null key part is refused outright rather than stored. No probe can
    /// build one (`KeyRef` has no null variant), so such a row would sit in the
    /// table unreachable — and the *second* blank-key row would then be counted
    /// as a duplicate of it, blaming the file for something it did not do.
    #[test]
    fn a_null_key_is_rejected_rather_than_stored_unreachably() {
        let key = hash_key_cfg(&["k"]);
        let names = [("k", ValueType::String), ("v", ValueType::String)];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        for v in ["unset", "also unset"] {
            assert_eq!(
                b.push_row(&Cells(&[CellRef::Null, CellRef::String(v)])),
                RowOutcome::Rejected(RowReject::NullKey)
            );
        }
        b.push_row(&Cells(&[CellRef::String("x"), CellRef::String("set")]));
        let data = b.finish(Duration::ZERO).unwrap();

        assert_eq!(data.rows(), 1, "only the usable row is stored");
        // both blank rows are blamed on the null key, not on each other
        assert_eq!(data.stats().skipped.null_key, 2);
        assert_eq!(data.stats().skipped.duplicate_key, 0);
        // an empty string is still a perfectly good key
        let empty = build(
            &names,
            &key,
            &[vec![CellRef::String(""), CellRef::String("blank")]],
        )
        .unwrap();
        let v = empty.schema().index_of("v").unwrap();
        let (row, _) = empty.lookup(Lookup::Hash(&[KeyRef::String("")])).unwrap();
        assert_eq!(empty.cell(row, v), CellRef::String("blank"));
    }

    #[test]
    fn duplicate_hash_keys_are_skipped_and_the_first_wins() {
        let key = hash_key_cfg(&["k"]);
        let names = [("k", ValueType::String), ("v", ValueType::String)];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        assert_eq!(
            b.push_row(&Cells(&[CellRef::String("a"), CellRef::String("first")])),
            RowOutcome::Inserted
        );
        assert_eq!(
            b.push_row(&Cells(&[CellRef::String("a"), CellRef::String("second")])),
            RowOutcome::Rejected(RowReject::DuplicateKey)
        );
        let data = b.finish(Duration::ZERO).unwrap();
        assert_eq!(data.stats().skipped.duplicate_key, 1);
        assert_eq!(data.rows(), 1);
        let v = data.schema().index_of("v").unwrap();
        let (row, _) = data.lookup(Lookup::Hash(&[KeyRef::String("a")])).unwrap();
        assert_eq!(data.cell(row, v), CellRef::String("first"));
    }

    #[test]
    fn identical_hash_rows_are_skipped_without_growing_the_index() {
        let key = hash_key_cfg(&["k"]);
        let names = [("k", ValueType::String), ("v", ValueType::String)];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        let cells = [CellRef::String("a"), CellRef::String("first")];
        assert_eq!(b.push_row(&Cells(&cells)), RowOutcome::Inserted);
        assert_eq!(
            b.push_row(&Cells(&cells)),
            RowOutcome::Rejected(RowReject::DuplicateKey)
        );

        let data = b.finish(Duration::ZERO).unwrap();
        assert_eq!(data.rows(), 1);
        assert_eq!(data.index_entries(), 1);
        assert_eq!(data.stats().skipped.duplicate_key, 1);
        let v = data.schema().index_of("v").unwrap();
        let (row, _) = data.lookup(Lookup::Hash(&[KeyRef::String("a")])).unwrap();
        assert_eq!(data.cell(row, v), CellRef::String("first"));
    }

    #[test]
    fn the_prefix_column_is_consumed_rather_than_stored() {
        let key = prefix_key_cfg(&[]);
        let names = [("network", ValueType::String), ("label", ValueType::String)];
        let data = build(
            &names,
            &key,
            &[vec![CellRef::String("10.0.0.0/8"), CellRef::String("low")]],
        )
        .unwrap();
        assert_eq!(data.schema().len(), 1);
        assert!(data.schema().index_of("network").is_none());
        assert!(data.schema().index_of("label").is_some());
    }

    #[test]
    fn many_prefixes_collapse_onto_one_deduplicated_row() {
        let key = prefix_key_cfg(&[]);
        let names = [
            ("network", ValueType::String),
            ("country", ValueType::String),
        ];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        for i in 0..50u8 {
            let cidr = format!("10.{i}.0.0/16");
            let outcome = b.push_row(&Cells(&[CellRef::String(&cidr), CellRef::String("ID")]));
            let want = if i == 0 {
                RowOutcome::Inserted
            } else {
                RowOutcome::Deduplicated
            };
            assert_eq!(outcome, want, "prefix {i}");
        }
        let data = b.finish(Duration::ZERO).unwrap();
        assert_eq!(data.rows(), 1, "all 50 prefixes share one row");
        assert_eq!(data.stats().unique_rows, 1);
        assert_eq!(data.stats().index_entries, 50);
        assert_eq!(data.stats().rows_read, 50);
        for i in 0..50u8 {
            assert_eq!(
                at_prefix(&data, &[], &format!("10.{i}.3.4"), "country"),
                Some((CellRef::String("ID"), net(&format!("10.{i}.0.0/16"))))
            );
        }
        assert_eq!(at_prefix(&data, &[], "10.50.0.1", "country"), None);
    }

    #[test]
    fn nested_prefixes_match_the_narrowest() {
        let key = prefix_key_cfg(&[]);
        let names = [("network", ValueType::String), ("label", ValueType::String)];
        let data = build(
            &names,
            &key,
            &[
                // wider after narrower: insertion order must not matter
                vec![CellRef::String("10.1.2.0/24"), CellRef::String("c")],
                vec![CellRef::String("10.0.0.0/8"), CellRef::String("a")],
                vec![CellRef::String("10.1.0.0/16"), CellRef::String("b")],
            ],
        )
        .unwrap();
        assert_eq!(data.rows(), 3);
        let at = |addr| at_prefix(&data, &[], addr, "label");
        assert_eq!(
            at("10.1.2.3"),
            Some((CellRef::String("c"), net("10.1.2.0/24")))
        );
        assert_eq!(
            at("10.1.3.3"),
            Some((CellRef::String("b"), net("10.1.0.0/16")))
        );
        assert_eq!(
            at("10.2.0.0"),
            Some((CellRef::String("a"), net("10.0.0.0/8")))
        );
        assert_eq!(at("11.0.0.0"), None);
    }

    #[test]
    fn duplicate_prefixes_are_skipped_without_leaving_an_orphan_row() {
        let key = prefix_key_cfg(&[]);
        let names = [("network", ValueType::String), ("label", ValueType::String)];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        b.push_row(&Cells(&[
            CellRef::String("10.0.0.0/8"),
            CellRef::String("a"),
        ]));
        assert_eq!(
            b.push_row(&Cells(&[
                CellRef::String("10.0.0.0/8"),
                CellRef::String("b")
            ])),
            RowOutcome::Rejected(RowReject::DuplicateKey)
        );
        let data = b.finish(Duration::ZERO).unwrap();
        assert_eq!(data.rows(), 1, "the rejected row was never materialized");
        assert_eq!(data.stats().skipped.duplicate_key, 1);
        assert_eq!(
            at_prefix(&data, &[], "10.0.0.7", "label"),
            Some((CellRef::String("a"), net("10.0.0.0/8")))
        );
    }

    #[test]
    fn unusable_prefixes_are_rejected() {
        let key = prefix_key_cfg(&[]);
        let names = [("network", ValueType::String), ("label", ValueType::String)];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        for bad in [
            CellRef::Null,
            CellRef::String("not a prefix"),
            CellRef::String("10.0.0.1/8"),
            CellRef::String("10.0.0.0/33"),
        ] {
            assert_eq!(
                b.push_row(&Cells(&[bad, CellRef::String("x")])),
                RowOutcome::Rejected(RowReject::BadPrefix),
                "{bad:?}"
            );
        }
        b.push_row(&Cells(&[
            CellRef::String("192.0.2.1"),
            CellRef::String("host"),
        ]));
        let data = b.finish(Duration::ZERO).unwrap();
        assert_eq!(data.stats().skipped.bad_prefix, 4);
        assert!(
            data.stats()
                .first_error
                .as_deref()
                .is_some_and(|e| e.contains("prefix")),
            "{:?}",
            data.stats().first_error
        );
        // a bare address is a host route
        assert_eq!(
            at_prefix(&data, &[], "192.0.2.1", "label"),
            Some((CellRef::String("host"), net("192.0.2.1/32")))
        );
        assert_eq!(at_prefix(&data, &[], "192.0.2.2", "label"), None);
    }

    #[test]
    fn prefixes_cover_both_families_and_ignore_the_configured_cast() {
        let key = prefix_key_cfg(&[]);
        // the prefix column is declared as an IP; `source_columns` must still
        // read it as text
        let names = [("network", ValueType::Ip), ("asn", ValueType::Integer)];
        let src = source(&names, &key);
        assert_eq!(
            src[0].1,
            ValueType::String,
            "prefix column type is overridden"
        );
        let data = build(
            &names,
            &key,
            &[
                vec![CellRef::String("10.0.0.0/24"), CellRef::Integer(64500)],
                vec![CellRef::String("2001:db8::/32"), CellRef::Integer(64501)],
                vec![
                    CellRef::String("::ffff:10.9.0.0/112"),
                    CellRef::Integer(64502),
                ],
            ],
        )
        .unwrap();
        let at = |addr| at_prefix(&data, &[], addr, "asn").map(|(c, _)| c);
        assert_eq!(at("10.0.0.7"), Some(CellRef::Integer(64500)));
        assert_eq!(at("2001:db8::5"), Some(CellRef::Integer(64501)));
        // a v4-mapped prefix is stored as v4
        assert_eq!(at("10.9.1.1"), Some(CellRef::Integer(64502)));
        assert_eq!(at("11.0.0.1"), None);
        assert_eq!(at("2001:dbf::1"), None);
    }

    #[test]
    fn partitioned_prefixes_do_not_collide() {
        let key = prefix_key_cfg(&["vrf"]);
        let names = [
            ("vrf", ValueType::String),
            ("network", ValueType::String),
            ("label", ValueType::String),
        ];
        let data = build(
            &names,
            &key,
            &[
                vec![
                    CellRef::String("red"),
                    CellRef::String("10.0.0.0/8"),
                    CellRef::String("red-net"),
                ],
                vec![
                    CellRef::String("blue"),
                    CellRef::String("10.0.0.0/8"),
                    CellRef::String("blue-net"),
                ],
            ],
        )
        .unwrap();
        let at = |vrf: &str| {
            at_prefix(&data, &[KeyRef::String(vrf)], "10.0.0.5", "label").map(|(c, _)| c)
        };
        assert_eq!(at("red"), Some(CellRef::String("red-net")));
        assert_eq!(at("blue"), Some(CellRef::String("blue-net")));
        assert_eq!(at("green"), None);
        // the partition column stays readable
        assert_eq!(
            at_prefix(&data, &[KeyRef::String("red")], "10.0.0.5", "vrf").map(|(c, _)| c),
            Some(CellRef::String("red"))
        );
        // and a probe naming no partition misses
        assert_eq!(at_prefix(&data, &[], "10.0.0.5", "label"), None);
    }

    #[test]
    fn probing_with_the_wrong_key_shape_is_a_miss() {
        let key = prefix_key_cfg(&[]);
        let names = [("network", ValueType::String), ("label", ValueType::String)];
        let data = build(
            &names,
            &key,
            &[vec![CellRef::String("10.0.0.0/8"), CellRef::String("a")]],
        )
        .unwrap();
        assert_eq!(data.lookup(Lookup::Hash(&[KeyRef::Integer(5)])), None);
    }

    #[test]
    fn a_load_where_every_row_fails_is_an_error() {
        let key = prefix_key_cfg(&[]);
        let names = [("network", ValueType::String), ("label", ValueType::String)];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        // every prefix is null
        for _ in 0..3 {
            b.push_row(&Cells(&[CellRef::Null, CellRef::String("x")]));
        }
        let err = b.finish(Duration::ZERO).unwrap_err().to_string();
        assert!(err.contains("every row was rejected"), "{err}");
        assert!(err.contains("prefix=3"), "{err}");
    }

    #[test]
    fn a_load_with_no_rows_at_all_is_an_empty_table_not_an_error() {
        let key = hash_key_cfg(&["k"]);
        let names = [("k", ValueType::String), ("v", ValueType::String)];
        let data = build(&names, &key, &[]).unwrap();
        assert_eq!(data.rows(), 0);
        assert_eq!(data.lookup(Lookup::Hash(&[KeyRef::String("a")])), None);
    }

    #[test]
    fn wrong_field_count_is_rejected() {
        let key = hash_key_cfg(&["k"]);
        let names = [("k", ValueType::String), ("v", ValueType::String)];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        assert_eq!(
            b.push_row(&Cells(&[CellRef::String("only-one")])),
            RowOutcome::Rejected(RowReject::Arity)
        );
        b.push_row(&Cells(&[CellRef::String("k"), CellRef::String("v")]));
        let data = b.finish(Duration::ZERO).unwrap();
        assert_eq!(data.stats().skipped.arity, 1);
        assert_eq!(data.rows(), 1);
    }

    #[test]
    fn an_empty_table_reports_no_rows() {
        let key = hash_key_cfg(&["k"]);
        let names = [("k", ValueType::String), ("v", ValueType::String)];
        let src = source(&names, &key);
        let (_, schema, plan) = ColumnLayout::build(&src, &key).unwrap();
        let data = ImmutableData::empty(Arc::new(schema), plan);
        assert_eq!(data.rows(), 0);
        assert_eq!(data.index_entries(), 0);
        assert_eq!(data.lookup(Lookup::Hash(&[KeyRef::String("a")])), None);
    }
}
