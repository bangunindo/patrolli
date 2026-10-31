// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::cell::{CellRef, KeyRef, Lookup, RangePoint, hash_key};
use super::column::{Column, ColumnBuilder, Interner, StagedCell};
use super::index::{HashIndex, Index, RangeIndexBuilder, RangeReject};
use super::schema::{ColumnIdx, ColumnLayout, KeyPlan, RowId, Schema};
use crate::config::lookup_table as cfg;
use anyhow::{Result, bail};
use hashbrown::HashTable;
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
    /// a range bound was null, or not the type `range_type` calls for
    BadBound,
    /// a key or partition column was null, which no probe can name
    NullKey,
    /// another row already claims this exact key
    DuplicateKey,
    /// the range collides with one already indexed
    Overlap,
    /// the range is reversed, or exclusive bounds left nothing between them
    EmptyRange,
    /// start and end are different IP families
    Mismatch,
}

impl RowReject {
    pub fn as_str(self) -> &'static str {
        match self {
            RowReject::Arity => "field count",
            RowReject::BadCell => "cast",
            RowReject::BadBound => "range bound",
            RowReject::NullKey => "null key",
            RowReject::DuplicateKey => "duplicate key",
            RowReject::Overlap => "overlapping range",
            RowReject::EmptyRange => "empty range",
            RowReject::Mismatch => "mixed address families",
        }
    }
}

impl From<RangeReject> for RowReject {
    fn from(r: RangeReject) -> Self {
        match r {
            RangeReject::Empty => RowReject::EmptyRange,
            RangeReject::Overlap => RowReject::Overlap,
            RangeReject::Mismatch => RowReject::Mismatch,
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
    #[serde(rename = "range_bound")]
    pub bad_bound: u64,
    pub null_key: u64,
    pub duplicate_key: u64,
    pub overlap: u64,
    pub empty_range: u64,
    #[serde(rename = "family_mismatch")]
    pub mismatch: u64,
}

impl SkipCounts {
    pub fn record(&mut self, reject: RowReject) {
        let slot = match reject {
            RowReject::Arity => &mut self.arity,
            RowReject::BadCell => &mut self.bad_cell,
            RowReject::BadBound => &mut self.bad_bound,
            RowReject::NullKey => &mut self.null_key,
            RowReject::DuplicateKey => &mut self.duplicate_key,
            RowReject::Overlap => &mut self.overlap,
            RowReject::EmptyRange => &mut self.empty_range,
            RowReject::Mismatch => &mut self.mismatch,
        };
        *slot += 1;
    }

    pub fn total(&self) -> u64 {
        self.arity
            + self.bad_cell
            + self.bad_bound
            + self.null_key
            + self.duplicate_key
            + self.overlap
            + self.empty_range
            + self.mismatch
    }

    /// A compact `reason=count` summary of the non-zero reasons.
    pub fn summary(&self) -> String {
        let parts = [
            ("field_count", self.arity),
            ("cast", self.bad_cell),
            ("range_bound", self.bad_bound),
            ("null_key", self.null_key),
            ("duplicate_key", self.duplicate_key),
            ("overlap", self.overlap),
            ("empty_range", self.empty_range),
            ("family_mismatch", self.mismatch),
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
            KeyPlan::Range {
                partition,
                range_type,
            } => Index::Range(RangeIndexBuilder::new(*range_type, !partition.is_empty()).finish()),
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
            cfg::Key::Range(range) => KeyPlan::Range {
                partition: Box::new([]),
                range_type: range.range_type,
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

    pub fn lookup(&self, key: Lookup<'_>) -> Option<RowId> {
        match (&self.index, key) {
            (Index::Hash(index), Lookup::Hash(parts)) => {
                let KeyPlan::Hash { columns } = &self.key else {
                    return None;
                };
                if parts.len() != columns.len() {
                    return None;
                }
                let hash = hash_key(parts.iter().copied());
                index.find(hash, |row| {
                    columns
                        .iter()
                        .zip(parts)
                        .all(|(col, part)| cell_matches_key(self.cell(row, *col), *part))
                })
            }
            (Index::Range(index), Lookup::Range { partition, point }) => {
                let KeyPlan::Range {
                    partition: cols, ..
                } = &self.key
                else {
                    return None;
                };
                if partition.len() != cols.len() {
                    return None;
                }
                index.get(partition, point)
            }
            // a hash probe against a range table, or the reverse
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

/// Resolves the source columns, forcing the range bound columns to the type
/// `range_type` demands regardless of any configured cast — they are parsed as
/// bounds, not as values.
pub fn source_columns(
    names: &[SmolStr],
    key: &cfg::Key,
    type_of: impl Fn(&str) -> cfg::ValueType,
) -> Vec<(SmolStr, cfg::ValueType)> {
    let bound_type = match key {
        cfg::Key::Range(range) => Some((
            range.start_column.as_str(),
            range.end_column.as_str(),
            match range.range_type {
                cfg::RangeType::Ip => cfg::ValueType::Ip,
                cfg::RangeType::Integer => cfg::ValueType::Integer,
            },
        )),
        cfg::Key::Hash(_) => None,
    };
    names
        .iter()
        .map(|name| {
            let ty = match bound_type {
                Some((start, end, ty)) if name == start || name == end => ty,
                _ => type_of(name),
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
    Range(RangeBuilder),
}

struct HashBuilder {
    key_columns: Box<[ColumnIdx]>,
    index: HashIndex,
}

struct RangeBuilder {
    partition: Box<[ColumnIdx]>,
    spec: RangeSpec,
    index: RangeIndexBuilder,
    /// row fingerprint -> row id; only range tables can share materialized rows
    dedup: HashTable<RowId>,
}

#[derive(Debug, Clone, Copy)]
struct RangeSpec {
    start: usize,
    end: usize,
    start_inclusive: bool,
    end_inclusive: bool,
    range_type: cfg::RangeType,
}

impl ImmutableBuilder {
    /// `source` is the column list exactly as it appears in the file, already
    /// typed by [`source_columns`].
    pub fn new(source: &[(SmolStr, cfg::ValueType)], key: &cfg::Key) -> Result<Self> {
        if source.is_empty() {
            bail!("table has no columns");
        }
        let (layout, schema, plan) = ColumnLayout::build(source, key)?;
        let state = match (key, plan, layout.range_start, layout.range_end) {
            (cfg::Key::Hash(_), KeyPlan::Hash { columns }, None, None) => {
                BuilderState::Hash(HashBuilder {
                    key_columns: columns,
                    index: HashIndex::default(),
                })
            }
            (
                cfg::Key::Range(range),
                KeyPlan::Range {
                    partition,
                    range_type,
                },
                Some(start),
                Some(end),
            ) => BuilderState::Range(RangeBuilder {
                index: RangeIndexBuilder::new(range_type, !partition.is_empty()),
                partition,
                spec: RangeSpec {
                    start,
                    end,
                    start_inclusive: range.start_inclusive,
                    end_inclusive: range.end_inclusive,
                    range_type,
                },
                dedup: HashTable::new(),
            }),
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
    /// including the range bound columns.
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
            BuilderState::Range(range) => range.push_row(rows, source),
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
            BuilderState::Range(range) => (
                KeyPlan::Range {
                    partition: range.partition,
                    range_type: range.spec.range_type,
                },
                Index::Range(range.index.finish()),
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

impl RangeBuilder {
    fn push_row(&mut self, rows: &mut RowsBuilder, source: &impl RowSource) -> RowOutcome {
        let row_no = rows.stats.rows_read;
        // Parse bounds before materialized cells so unusable bounds are cheap
        // to reject. A range builder always has its spec and index together.
        let spec = self.spec;
        let ty = match spec.range_type {
            cfg::RangeType::Ip => cfg::ValueType::Ip,
            cfg::RangeType::Integer => cfg::ValueType::Integer,
        };
        let start = source
            .cell(spec.start, ty)
            .ok()
            .and_then(|c| RangePoint::from_cell(c, spec.range_type));
        let end = source
            .cell(spec.end, ty)
            .ok()
            .and_then(|c| RangePoint::from_cell(c, spec.range_type));
        let (Some(start), Some(end)) = (start, end) else {
            return rows.reject(RowReject::BadBound, || {
                format!("row {row_no} has an unusable range bound")
            });
        };
        let staged = match rows.stage(source) {
            Ok(staged) => staged,
            Err(outcome) => return outcome,
        };

        // Only ranges can share a materialized row while naming distinct keys.
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
        if let Err(reject) = self.index.insert(
            &parts,
            start,
            spec.start_inclusive,
            end,
            spec.end_inclusive,
            row_id,
        ) {
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
    use crate::config::lookup_table::{Hash as HashKey, Range as RangeKey, RangeType, ValueType};

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
    fn rejected_ranges_retain_neither_strings_nor_empty_partitions() {
        let key = range_key_cfg(RangeType::Integer, &["host"], true, true);
        let columns = source(
            &[
                ("host", ValueType::String),
                ("start", ValueType::Integer),
                ("end", ValueType::Integer),
                ("site", ValueType::String),
            ],
            &key,
        );
        let mut builder = ImmutableBuilder::new(&columns, &key).unwrap();
        builder.push_row(&Cells(&[
            CellRef::String("a"),
            CellRef::Integer(1),
            CellRef::Integer(10),
            CellRef::String("kept"),
        ]));
        for i in 0..1000 {
            let text = format!("rejected-{i}");
            assert_eq!(
                builder.push_row(&Cells(&[
                    CellRef::String("a"),
                    CellRef::Integer(2),
                    CellRef::Integer(3),
                    CellRef::String(&text)
                ])),
                RowOutcome::Rejected(RowReject::Overlap)
            );
            assert_eq!(
                builder.push_row(&Cells(&[
                    CellRef::String(&text),
                    CellRef::Integer(20),
                    CellRef::Integer(10),
                    CellRef::String(&text)
                ])),
                RowOutcome::Rejected(RowReject::EmptyRange)
            );
        }
        let data = builder.finish(Duration::ZERO).unwrap();
        assert_eq!(data.interned_strings(), 2);
        let Index::Range(index) = &data.index else {
            unreachable!()
        };
        assert_eq!(index.partitions(), 1);
    }

    fn range_key_cfg(
        range_type: RangeType,
        partition: &[&str],
        start_inclusive: bool,
        end_inclusive: bool,
    ) -> cfg::Key {
        cfg::Key::Range(RangeKey {
            range_type,
            partition_columns: partition.iter().map(|s| s.to_string()).collect(),
            start_column: "start".into(),
            start_inclusive,
            end_column: "end".into(),
            end_inclusive,
        })
    }

    /// Resolves source columns the way the loader does, so the range bound
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
                .map(|row| data.cell(row, city))
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
        let row = empty.lookup(Lookup::Hash(&[KeyRef::String("")])).unwrap();
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
        let row = data.lookup(Lookup::Hash(&[KeyRef::String("a")])).unwrap();
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
        let row = data.lookup(Lookup::Hash(&[KeyRef::String("a")])).unwrap();
        assert_eq!(data.cell(row, v), CellRef::String("first"));
    }

    #[test]
    fn range_bounds_are_consumed_rather_than_stored() {
        let key = range_key_cfg(RangeType::Integer, &[], true, true);
        let names = [
            ("start", ValueType::Integer),
            ("end", ValueType::Integer),
            ("label", ValueType::String),
        ];
        let data = build(
            &names,
            &key,
            &[vec![
                CellRef::Integer(1),
                CellRef::Integer(10),
                CellRef::String("low"),
            ]],
        )
        .unwrap();
        assert_eq!(data.schema().len(), 1);
        assert!(data.schema().index_of("start").is_none());
        assert!(data.schema().index_of("end").is_none());
        assert!(data.schema().index_of("label").is_some());
    }

    #[test]
    fn many_ranges_collapse_onto_one_deduplicated_row() {
        let key = range_key_cfg(RangeType::Integer, &[], true, true);
        let names = [
            ("start", ValueType::Integer),
            ("end", ValueType::Integer),
            ("country", ValueType::String),
        ];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        // 50 disjoint ranges with a one-value gap between them, all naming the
        // same country. The gap keeps them from coalescing, so this isolates
        // row deduplication from range coalescing.
        for i in 0..50i64 {
            let outcome = b.push_row(&Cells(&[
                CellRef::Integer(i * 10),
                CellRef::Integer(i * 10 + 8),
                CellRef::String("ID"),
            ]));
            let want = if i == 0 {
                RowOutcome::Inserted
            } else {
                RowOutcome::Deduplicated
            };
            assert_eq!(outcome, want, "range {i}");
        }
        let data = b.finish(Duration::ZERO).unwrap();
        assert_eq!(data.rows(), 1, "all 50 ranges share one row");
        assert_eq!(data.stats().unique_rows, 1);
        assert_eq!(data.stats().index_entries, 50);
        assert_eq!(data.stats().rows_read, 50);

        let country = data.schema().index_of("country").unwrap();
        for i in 0..50i64 {
            let row = data
                .lookup(Lookup::Range {
                    partition: &[],
                    point: RangePoint::Integer(i * 10 + 5),
                })
                .unwrap();
            assert_eq!(data.cell(row, country), CellRef::String("ID"));
        }
        assert_eq!(
            data.lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(-1)
            }),
            None
        );
        // and the gaps really are gaps
        assert_eq!(
            data.lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(9)
            }),
            None
        );
    }

    #[test]
    fn adjacent_ranges_naming_the_same_row_coalesce() {
        // consecutive geoip blocks assigned to one country collapse into a
        // single index entry, on top of sharing a single stored row
        let key = range_key_cfg(RangeType::Integer, &[], true, true);
        let names = [
            ("start", ValueType::Integer),
            ("end", ValueType::Integer),
            ("country", ValueType::String),
        ];
        let data = build(
            &names,
            &key,
            &[
                vec![
                    CellRef::Integer(0),
                    CellRef::Integer(9),
                    CellRef::String("ID"),
                ],
                vec![
                    CellRef::Integer(10),
                    CellRef::Integer(19),
                    CellRef::String("ID"),
                ],
                vec![
                    CellRef::Integer(20),
                    CellRef::Integer(29),
                    CellRef::String("ID"),
                ],
            ],
        )
        .unwrap();
        assert_eq!(data.rows(), 1);
        assert_eq!(
            data.stats().index_entries,
            1,
            "three adjacent ranges merged"
        );
        let country = data.schema().index_of("country").unwrap();
        for point in [0, 15, 29] {
            let row = data
                .lookup(Lookup::Range {
                    partition: &[],
                    point: RangePoint::Integer(point),
                })
                .unwrap();
            assert_eq!(data.cell(row, country), CellRef::String("ID"));
        }
        assert_eq!(
            data.lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(30)
            }),
            None
        );
    }

    #[test]
    fn distinct_rows_are_kept_apart() {
        let key = range_key_cfg(RangeType::Integer, &[], true, true);
        let names = [
            ("start", ValueType::Integer),
            ("end", ValueType::Integer),
            ("country", ValueType::String),
        ];
        let data = build(
            &names,
            &key,
            &[
                vec![
                    CellRef::Integer(0),
                    CellRef::Integer(9),
                    CellRef::String("ID"),
                ],
                vec![
                    CellRef::Integer(10),
                    CellRef::Integer(19),
                    CellRef::String("MY"),
                ],
                vec![
                    CellRef::Integer(20),
                    CellRef::Integer(29),
                    CellRef::String("ID"),
                ],
            ],
        )
        .unwrap();
        assert_eq!(data.rows(), 2);
        let country = data.schema().index_of("country").unwrap();
        let at = |p: i64| {
            data.lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(p),
            })
            .map(|r| data.cell(r, country))
        };
        assert_eq!(at(5), Some(CellRef::String("ID")));
        assert_eq!(at(15), Some(CellRef::String("MY")));
        assert_eq!(at(25), Some(CellRef::String("ID")));
    }

    #[test]
    fn overlapping_ranges_are_skipped_without_leaving_an_orphan_row() {
        let key = range_key_cfg(RangeType::Integer, &[], true, true);
        let names = [
            ("start", ValueType::Integer),
            ("end", ValueType::Integer),
            ("label", ValueType::String),
        ];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        b.push_row(&Cells(&[
            CellRef::Integer(0),
            CellRef::Integer(10),
            CellRef::String("a"),
        ]));
        assert_eq!(
            b.push_row(&Cells(&[
                CellRef::Integer(5),
                CellRef::Integer(15),
                CellRef::String("b")
            ])),
            RowOutcome::Rejected(RowReject::Overlap)
        );
        let data = b.finish(Duration::ZERO).unwrap();
        assert_eq!(data.rows(), 1, "the rejected row was never materialized");
        assert_eq!(data.stats().skipped.overlap, 1);
        let label = data.schema().index_of("label").unwrap();
        let row = data
            .lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(7),
            })
            .unwrap();
        assert_eq!(data.cell(row, label), CellRef::String("a"));
    }

    #[test]
    fn ip_ranges_cover_both_families_and_ignore_the_configured_cast() {
        let key = range_key_cfg(RangeType::Ip, &[], true, true);
        // the bound columns are declared as strings; `source_columns` must
        // still parse them as addresses
        let names = [
            ("start", ValueType::String),
            ("end", ValueType::String),
            ("asn", ValueType::Integer),
        ];
        let src = source(&names, &key);
        assert_eq!(src[0].1, ValueType::Ip, "bound column type is overridden");
        let ip = |s: &str| CellRef::Ip(s.parse().unwrap());
        let data = build(
            &names,
            &key,
            &[
                vec![ip("10.0.0.0"), ip("10.0.0.255"), CellRef::Integer(64500)],
                vec![
                    ip("2001:db8::"),
                    ip("2001:db8::ffff"),
                    CellRef::Integer(64501),
                ],
            ],
        )
        .unwrap();
        let asn = data.schema().index_of("asn").unwrap();
        let at = |s: &str| {
            data.lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Ip(s.parse().unwrap()),
            })
            .map(|r| data.cell(r, asn))
        };
        assert_eq!(at("10.0.0.7"), Some(CellRef::Integer(64500)));
        assert_eq!(at("2001:db8::5"), Some(CellRef::Integer(64501)));
        assert_eq!(at("11.0.0.1"), None);
        assert_eq!(at("2001:dbf::1"), None);
    }

    #[test]
    fn partitioned_ranges_do_not_collide() {
        let key = range_key_cfg(RangeType::Integer, &["vrf"], true, true);
        let names = [
            ("vrf", ValueType::String),
            ("start", ValueType::Integer),
            ("end", ValueType::Integer),
            ("label", ValueType::String),
        ];
        let data = build(
            &names,
            &key,
            &[
                vec![
                    CellRef::String("red"),
                    CellRef::Integer(1),
                    CellRef::Integer(10),
                    CellRef::String("red-low"),
                ],
                vec![
                    CellRef::String("blue"),
                    CellRef::Integer(1),
                    CellRef::Integer(10),
                    CellRef::String("blue-low"),
                ],
            ],
        )
        .unwrap();
        let label = data.schema().index_of("label").unwrap();
        let at = |vrf: &str| {
            data.lookup(Lookup::Range {
                partition: &[KeyRef::String(vrf)],
                point: RangePoint::Integer(5),
            })
            .map(|r| data.cell(r, label))
        };
        assert_eq!(at("red"), Some(CellRef::String("red-low")));
        assert_eq!(at("blue"), Some(CellRef::String("blue-low")));
        assert_eq!(at("green"), None);
        // the partition column stays readable
        let vrf = data.schema().index_of("vrf").unwrap();
        let row = data
            .lookup(Lookup::Range {
                partition: &[KeyRef::String("red")],
                point: RangePoint::Integer(5),
            })
            .unwrap();
        assert_eq!(data.cell(row, vrf), CellRef::String("red"));
    }

    #[test]
    fn exclusive_bounds_are_honoured_end_to_end() {
        // the common half-open convention: [start, end)
        let key = range_key_cfg(RangeType::Integer, &[], true, false);
        let names = [
            ("start", ValueType::Integer),
            ("end", ValueType::Integer),
            ("label", ValueType::String),
        ];
        let data = build(
            &names,
            &key,
            &[
                vec![
                    CellRef::Integer(0),
                    CellRef::Integer(10),
                    CellRef::String("a"),
                ],
                vec![
                    CellRef::Integer(10),
                    CellRef::Integer(20),
                    CellRef::String("b"),
                ],
            ],
        )
        .unwrap();
        let label = data.schema().index_of("label").unwrap();
        let at = |p: i64| {
            data.lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Integer(p),
            })
            .map(|r| data.cell(r, label))
        };
        assert_eq!(at(9), Some(CellRef::String("a")));
        assert_eq!(at(10), Some(CellRef::String("b")), "end is exclusive");
        assert_eq!(at(19), Some(CellRef::String("b")));
        assert_eq!(at(20), None);
    }

    #[test]
    fn probing_with_the_wrong_key_shape_is_a_miss() {
        let key = range_key_cfg(RangeType::Integer, &[], true, true);
        let names = [
            ("start", ValueType::Integer),
            ("end", ValueType::Integer),
            ("label", ValueType::String),
        ];
        let data = build(
            &names,
            &key,
            &[vec![
                CellRef::Integer(0),
                CellRef::Integer(10),
                CellRef::String("a"),
            ]],
        )
        .unwrap();
        assert_eq!(data.lookup(Lookup::Hash(&[KeyRef::Integer(5)])), None);
        // and a range probe with the wrong point type
        assert_eq!(
            data.lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Ip("10.0.0.1".parse().unwrap())
            }),
            None
        );
    }

    #[test]
    fn a_load_where_every_row_fails_is_an_error() {
        let key = range_key_cfg(RangeType::Integer, &[], true, true);
        let names = [
            ("start", ValueType::Integer),
            ("end", ValueType::Integer),
            ("label", ValueType::String),
        ];
        let src = source(&names, &key);
        let mut b = ImmutableBuilder::new(&src, &key).unwrap();
        // every bound is null
        for _ in 0..3 {
            b.push_row(&Cells(&[
                CellRef::Null,
                CellRef::Null,
                CellRef::String("x"),
            ]));
        }
        let err = b.finish(Duration::ZERO).unwrap_err().to_string();
        assert!(err.contains("every row was rejected"), "{err}");
        assert!(err.contains("range_bound=3"), "{err}");
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
