// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::cell::CellRef;
use crate::config::lookup_table::ValueType;
use anyhow::{Result, bail};
use std::net::{IpAddr, Ipv4Addr};
use string_interner::StringInterner;
use string_interner::backend::StringBackend;
use string_interner::symbol::SymbolU32;

pub type Interner = StringInterner<StringBackend<SymbolU32>>;

/// A packed bit vector. Used for booleans and for null masks, where one bit per
/// row is the difference between a rounding error and gigabytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bitmap {
    words: Box<[u64]>,
    len: usize,
}

impl Bitmap {
    #[inline]
    pub fn get(&self, i: usize) -> bool {
        self.words
            .get(i / 64)
            .is_some_and(|w| w & (1u64 << (i % 64)) != 0)
    }

    /// Bits pushed, as opposed to the bits the final word has room for.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Debug, Default)]
pub struct BitmapBuilder {
    words: Vec<u64>,
    len: usize,
    any: bool,
}

impl BitmapBuilder {
    pub fn push(&mut self, bit: bool) {
        if self.len.is_multiple_of(64) {
            self.words.push(0);
        }
        if bit {
            let last = self.words.len() - 1;
            self.words[last] |= 1u64 << (self.len % 64);
            self.any = true;
        }
        self.len += 1;
    }

    #[inline]
    pub fn get(&self, i: usize) -> bool {
        self.words
            .get(i / 64)
            .is_some_and(|w| w & (1u64 << (i % 64)) != 0)
    }

    pub fn any(&self) -> bool {
        self.any
    }

    pub fn finish(self) -> Bitmap {
        Bitmap {
            words: self.words.into_boxed_slice(),
            len: self.len,
        }
    }

    /// `None` when no bit is set, so a column with no nulls carries no mask.
    pub fn finish_if_any(self) -> Option<Bitmap> {
        self.any.then(|| self.finish())
    }
}

/// A cell after string interning: `Copy`, `Hash` and `Eq`, so a whole row can
/// be fingerprinted for deduplication. Doubles are keyed by their bit pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StagedCell {
    Null,
    Bool(bool),
    Double(u64),
    Integer(i64),
    String(SymbolU32),
    Ip(IpAddr),
}

impl StagedCell {
    /// Interns any string, turning a borrowed cell into a comparable one.
    pub fn stage(cell: CellRef<'_>, strings: &mut Interner) -> Self {
        match cell {
            CellRef::Null => StagedCell::Null,
            CellRef::Bool(v) => StagedCell::Bool(v),
            CellRef::Double(v) => StagedCell::Double(v.to_bits()),
            CellRef::Integer(v) => StagedCell::Integer(v),
            CellRef::String(v) => StagedCell::String(strings.get_or_intern(v)),
            CellRef::Ip(v) => StagedCell::Ip(v),
        }
    }
}

/// One materialized column, stored as a typed array plus an optional null mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Column {
    Bool {
        vals: Bitmap,
        nulls: Option<Bitmap>,
    },
    Double {
        vals: Box<[u64]>, // bit patterns, so the column stays `Eq`
        nulls: Option<Bitmap>,
    },
    Integer {
        vals: Box<[i64]>,
        nulls: Option<Bitmap>,
    },
    /// `SymbolU32` is `NonZeroU32`-backed, so `Option` is still 4 bytes and the
    /// null mask comes for free.
    String {
        syms: Box<[Option<SymbolU32>]>,
    },
    Ip {
        vals: Box<[IpAddr]>,
        nulls: Option<Bitmap>,
    },
}

impl Column {
    #[inline]
    pub fn get<'a>(&'a self, row: usize, strings: &'a Interner) -> CellRef<'a> {
        let null = |nulls: &Option<Bitmap>| nulls.as_ref().is_some_and(|n| n.get(row));
        match self {
            // `row >= vals.len()` is Null, not `Bool(false)`: every other
            // variant reads an absent row as Null, and a bitmap's `get` cannot
            // tell a cleared bit from one past the end on its own.
            Column::Bool { vals, nulls } => {
                if row >= vals.len() || null(nulls) {
                    CellRef::Null
                } else {
                    CellRef::Bool(vals.get(row))
                }
            }
            Column::Double { vals, nulls } => match vals.get(row) {
                Some(v) if !null(nulls) => CellRef::Double(f64::from_bits(*v)),
                _ => CellRef::Null,
            },
            Column::Integer { vals, nulls } => match vals.get(row) {
                Some(v) if !null(nulls) => CellRef::Integer(*v),
                _ => CellRef::Null,
            },
            Column::String { syms } => match syms.get(row).copied().flatten() {
                Some(sym) => strings.resolve(sym).map_or(CellRef::Null, CellRef::String),
                None => CellRef::Null,
            },
            Column::Ip { vals, nulls } => match vals.get(row) {
                Some(v) if !null(nulls) => CellRef::Ip(*v),
                _ => CellRef::Null,
            },
        }
    }
}

#[derive(Debug)]
pub enum ColumnBuilder {
    Bool {
        vals: BitmapBuilder,
        nulls: BitmapBuilder,
    },
    Double {
        vals: Vec<u64>,
        nulls: BitmapBuilder,
    },
    Integer {
        vals: Vec<i64>,
        nulls: BitmapBuilder,
    },
    String {
        syms: Vec<Option<SymbolU32>>,
    },
    Ip {
        vals: Vec<IpAddr>,
        nulls: BitmapBuilder,
    },
}

impl ColumnBuilder {
    pub fn new(ty: ValueType) -> Self {
        match ty {
            ValueType::Bool => ColumnBuilder::Bool {
                vals: BitmapBuilder::default(),
                nulls: BitmapBuilder::default(),
            },
            ValueType::Double => ColumnBuilder::Double {
                vals: Vec::new(),
                nulls: BitmapBuilder::default(),
            },
            ValueType::Integer => ColumnBuilder::Integer {
                vals: Vec::new(),
                nulls: BitmapBuilder::default(),
            },
            ValueType::String => ColumnBuilder::String { syms: Vec::new() },
            ValueType::Ip => ColumnBuilder::Ip {
                vals: Vec::new(),
                nulls: BitmapBuilder::default(),
            },
        }
    }

    pub fn push(&mut self, cell: StagedCell) -> Result<()> {
        match (self, cell) {
            (ColumnBuilder::Bool { vals, nulls }, StagedCell::Bool(v)) => {
                vals.push(v);
                nulls.push(false);
            }
            (ColumnBuilder::Bool { vals, nulls }, StagedCell::Null) => {
                vals.push(false);
                nulls.push(true);
            }
            (ColumnBuilder::Double { vals, nulls }, StagedCell::Double(v)) => {
                vals.push(v);
                nulls.push(false);
            }
            (ColumnBuilder::Double { vals, nulls }, StagedCell::Null) => {
                vals.push(0);
                nulls.push(true);
            }
            (ColumnBuilder::Integer { vals, nulls }, StagedCell::Integer(v)) => {
                vals.push(v);
                nulls.push(false);
            }
            (ColumnBuilder::Integer { vals, nulls }, StagedCell::Null) => {
                vals.push(0);
                nulls.push(true);
            }
            (ColumnBuilder::String { syms }, StagedCell::String(v)) => syms.push(Some(v)),
            (ColumnBuilder::String { syms }, StagedCell::Null) => syms.push(None),
            (ColumnBuilder::Ip { vals, nulls }, StagedCell::Ip(v)) => {
                vals.push(v);
                nulls.push(false);
            }
            (ColumnBuilder::Ip { vals, nulls }, StagedCell::Null) => {
                vals.push(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
                nulls.push(true);
            }
            (builder, cell) => bail!(
                "cannot store a {} in a {} column",
                cell_kind(cell),
                builder.kind()
            ),
        }
        Ok(())
    }

    /// Reads a row back during the build, so row deduplication can compare a
    /// candidate against an already-stored row without keeping a second copy.
    pub fn get(&self, row: usize) -> StagedCell {
        match self {
            ColumnBuilder::Bool { vals, nulls } => {
                if nulls.get(row) {
                    StagedCell::Null
                } else {
                    StagedCell::Bool(vals.get(row))
                }
            }
            ColumnBuilder::Double { vals, nulls } => match vals.get(row) {
                Some(v) if !nulls.get(row) => StagedCell::Double(*v),
                _ => StagedCell::Null,
            },
            ColumnBuilder::Integer { vals, nulls } => match vals.get(row) {
                Some(v) if !nulls.get(row) => StagedCell::Integer(*v),
                _ => StagedCell::Null,
            },
            ColumnBuilder::String { syms } => match syms.get(row).copied().flatten() {
                Some(sym) => StagedCell::String(sym),
                None => StagedCell::Null,
            },
            ColumnBuilder::Ip { vals, nulls } => match vals.get(row) {
                Some(v) if !nulls.get(row) => StagedCell::Ip(*v),
                _ => StagedCell::Null,
            },
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            ColumnBuilder::Bool { .. } => "bool",
            ColumnBuilder::Double { .. } => "double",
            ColumnBuilder::Integer { .. } => "integer",
            ColumnBuilder::String { .. } => "string",
            ColumnBuilder::Ip { .. } => "ip",
        }
    }

    pub fn finish(self) -> Column {
        match self {
            ColumnBuilder::Bool { vals, nulls } => Column::Bool {
                vals: vals.finish(),
                nulls: nulls.finish_if_any(),
            },
            ColumnBuilder::Double { vals, nulls } => Column::Double {
                vals: vals.into_boxed_slice(),
                nulls: nulls.finish_if_any(),
            },
            ColumnBuilder::Integer { vals, nulls } => Column::Integer {
                vals: vals.into_boxed_slice(),
                nulls: nulls.finish_if_any(),
            },
            ColumnBuilder::String { syms } => Column::String {
                syms: syms.into_boxed_slice(),
            },
            ColumnBuilder::Ip { vals, nulls } => Column::Ip {
                vals: vals.into_boxed_slice(),
                nulls: nulls.finish_if_any(),
            },
        }
    }
}

fn cell_kind(cell: StagedCell) -> &'static str {
    match cell {
        StagedCell::Null => "null",
        StagedCell::Bool(_) => "bool",
        StagedCell::Double(_) => "double",
        StagedCell::Integer(_) => "integer",
        StagedCell::String(_) => "string",
        StagedCell::Ip(_) => "ip",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant reads a row past the end as Null. A bitmap cannot tell a
    /// cleared bit from one past the end, so `Bool` needs its own length check
    /// to agree with the others.
    #[test]
    fn a_row_past_the_end_reads_as_null_for_every_column_kind() {
        let strings = Interner::new();
        for ty in [
            ValueType::Bool,
            ValueType::Double,
            ValueType::Integer,
            ValueType::String,
            ValueType::Ip,
        ] {
            let mut b = ColumnBuilder::new(ty);
            b.push(StagedCell::Null).unwrap();
            let column = b.finish();
            assert!(
                column.get(1, &strings).is_null(),
                "{ty:?} must read row 1 of a one-row column as null"
            );
        }
    }

    #[test]
    fn bitmap_round_trips_across_word_boundaries() {
        let mut b = BitmapBuilder::default();
        let pattern: Vec<bool> = (0..200).map(|i| i % 7 == 0).collect();
        for bit in &pattern {
            b.push(*bit);
        }
        assert!(b.any());
        let map = b.finish();
        for (i, want) in pattern.iter().enumerate() {
            assert_eq!(map.get(i), *want, "bit {i}");
        }
        assert!(!map.get(1000));
    }

    #[test]
    fn a_column_without_nulls_carries_no_mask() {
        let mut b = BitmapBuilder::default();
        for _ in 0..100 {
            b.push(false);
        }
        assert!(b.finish_if_any().is_none());
    }

    #[test]
    fn columns_round_trip_every_type() {
        let mut strings = Interner::new();
        let cases: Vec<(ValueType, Vec<CellRef<'static>>)> = vec![
            (
                ValueType::Bool,
                vec![CellRef::Bool(true), CellRef::Null, CellRef::Bool(false)],
            ),
            (
                ValueType::Double,
                vec![CellRef::Double(1.5), CellRef::Null, CellRef::Double(-2.0)],
            ),
            (
                ValueType::Integer,
                vec![CellRef::Integer(7), CellRef::Null, CellRef::Integer(-7)],
            ),
            (
                ValueType::String,
                vec![CellRef::String("a"), CellRef::Null, CellRef::String("bb")],
            ),
            (
                ValueType::Ip,
                vec![
                    CellRef::Ip("10.0.0.1".parse().unwrap()),
                    CellRef::Null,
                    CellRef::Ip("2001:db8::1".parse().unwrap()),
                ],
            ),
        ];
        for (ty, cells) in cases {
            let mut builder = ColumnBuilder::new(ty);
            for cell in &cells {
                builder
                    .push(StagedCell::stage(*cell, &mut strings))
                    .unwrap();
            }
            let column = builder.finish();
            for (row, want) in cells.iter().enumerate() {
                assert_eq!(column.get(row, &strings), *want, "{ty:?} row {row}");
            }
        }
    }

    #[test]
    fn pushing_the_wrong_type_errors() {
        let mut strings = Interner::new();
        let mut builder = ColumnBuilder::new(ValueType::Integer);
        let err = builder
            .push(StagedCell::stage(CellRef::String("x"), &mut strings))
            .unwrap_err();
        assert!(err.to_string().contains("integer"), "{err}");
    }

    #[test]
    fn builder_reads_back_what_it_stored() {
        let mut strings = Interner::new();
        let mut builder = ColumnBuilder::new(ValueType::String);
        let staged = StagedCell::stage(CellRef::String("kuala lumpur"), &mut strings);
        builder.push(staged).unwrap();
        builder.push(StagedCell::Null).unwrap();
        assert_eq!(builder.get(0), staged);
        assert_eq!(builder.get(1), StagedCell::Null);
    }

    #[test]
    fn interning_deduplicates_repeated_strings() {
        let mut strings = Interner::new();
        let a = StagedCell::stage(CellRef::String("jakarta"), &mut strings);
        let b = StagedCell::stage(CellRef::String("jakarta"), &mut strings);
        assert_eq!(a, b);
        assert_eq!(strings.len(), 1);
    }
}
