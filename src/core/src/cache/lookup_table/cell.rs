// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::config::lookup_table::{RangeType, ValueType};
use crate::payload::{Value, ZString};
use anyhow::{Result, bail};
use smol_str::SmolStr;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use xxhash_rust::xxh3::Xxh3;

/// An owned cell, used by mutable tables.
#[derive(Debug, Clone, PartialEq)]
pub enum CellOwned {
    Null,
    Bool(bool),
    Double(f64),
    Integer(i64),
    String(SmolStr),
    Ip(IpAddr),
}

impl CellOwned {
    pub fn as_ref(&self) -> CellRef<'_> {
        match self {
            CellOwned::Null => CellRef::Null,
            CellOwned::Bool(v) => CellRef::Bool(*v),
            CellOwned::Double(v) => CellRef::Double(*v),
            CellOwned::Integer(v) => CellRef::Integer(*v),
            CellOwned::String(v) => CellRef::String(v.as_str()),
            CellOwned::Ip(v) => CellRef::Ip(*v),
        }
    }
}

/// A borrowed cell. Every read path hands these out, so a string lookup costs
/// no allocation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CellRef<'a> {
    Null,
    Bool(bool),
    Double(f64),
    Integer(i64),
    String(&'a str),
    Ip(IpAddr),
}

impl<'a> CellRef<'a> {
    /// Bridges into the telemetry value type.
    ///
    /// Neither `payload::Value` nor the OTAP `LiteralValue` has an IP variant,
    /// so an IP-typed column stringifies here. Every other variant is free.
    pub fn to_value(self) -> Value<'a> {
        match self {
            CellRef::Null => Value::Null,
            CellRef::Bool(v) => Value::Bool(v),
            CellRef::Double(v) => Value::Double(v),
            CellRef::Integer(v) => Value::Int64(v),
            CellRef::String(v) => Value::String(ZString::from_ref(v)),
            CellRef::Ip(v) => Value::String(ZString::from_owned(v.to_string())),
        }
    }

    pub fn is_null(self) -> bool {
        matches!(self, CellRef::Null)
    }

    pub fn as_str(self) -> Option<&'a str> {
        match self {
            CellRef::String(v) => Some(v),
            _ => None,
        }
    }

    pub fn to_owned_cell(self) -> CellOwned {
        match self {
            CellRef::Null => CellOwned::Null,
            CellRef::Bool(v) => CellOwned::Bool(v),
            CellRef::Double(v) => CellOwned::Double(v),
            CellRef::Integer(v) => CellOwned::Integer(v),
            CellRef::String(v) => CellOwned::String(SmolStr::new(v)),
            CellRef::Ip(v) => CellOwned::Ip(v),
        }
    }

    /// Parses a raw text field as `ty`. Nullness is the caller's business —
    /// a source applies its own `null_values` first — so an empty field parses
    /// as the empty string for `String` and as an error for everything else.
    pub fn parse(raw: &'a str, ty: ValueType) -> Result<Self> {
        Ok(match ty {
            ValueType::String => CellRef::String(raw),
            ValueType::Integer => CellRef::Integer(
                raw.parse::<i64>()
                    .map_err(|e| anyhow::anyhow!("{raw:?} is not an integer: {e}"))?,
            ),
            ValueType::Double => CellRef::Double(
                raw.parse::<f64>()
                    .map_err(|e| anyhow::anyhow!("{raw:?} is not a double: {e}"))?,
            ),
            ValueType::Bool => CellRef::Bool(parse_bool(raw)?),
            ValueType::Ip => CellRef::Ip(parse_ip(raw)?),
        })
    }
}

fn parse_bool(raw: &str) -> Result<bool> {
    match raw {
        "1" => return Ok(true),
        "0" => return Ok(false),
        _ => {}
    }
    if raw.eq_ignore_ascii_case("true")
        || raw.eq_ignore_ascii_case("t")
        || raw.eq_ignore_ascii_case("yes")
    {
        return Ok(true);
    }
    if raw.eq_ignore_ascii_case("false")
        || raw.eq_ignore_ascii_case("f")
        || raw.eq_ignore_ascii_case("no")
    {
        return Ok(false);
    }
    bail!("{raw:?} is not a boolean")
}

/// Accepts both textual addresses and the bare-integer form some IP range
/// dumps ship (`u32` is read as v4, anything larger as v6).
pub fn parse_ip(raw: &str) -> Result<IpAddr> {
    if let Ok(ip) = raw.parse::<IpAddr>() {
        return Ok(canonical(ip));
    }
    if let Ok(v) = raw.parse::<u32>() {
        return Ok(IpAddr::V4(Ipv4Addr::from(v)));
    }
    if let Ok(v) = raw.parse::<u128>() {
        return Ok(canonical(IpAddr::V6(Ipv6Addr::from(v))));
    }
    bail!("{raw:?} is not an IP address")
}

/// Folds `::ffff:a.b.c.d` onto the v4 family so build and probe agree.
pub fn canonical(ip: IpAddr) -> IpAddr {
    ip.to_canonical()
}

/// A key part.
///
/// Two cell variants are deliberately absent. A double has no total order to
/// index on. A null is unprobeable — `attribute_lookup::key_part` refuses
/// `Value::Null` — so leaving the variant out is what stops a row being stored
/// under a key nothing could ever reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyRef<'a> {
    Bool(bool),
    Integer(i64),
    String(&'a str),
    Ip(IpAddr),
}

impl<'a> KeyRef<'a> {
    pub fn to_owned_key(self) -> KeyOwned {
        match self {
            KeyRef::Bool(v) => KeyOwned::Bool(v),
            KeyRef::Integer(v) => KeyOwned::Integer(v),
            KeyRef::String(v) => KeyOwned::String(SmolStr::new(v)),
            KeyRef::Ip(v) => KeyOwned::Ip(v),
        }
    }

    pub fn try_from_cell(cell: CellRef<'a>) -> Result<Self> {
        Ok(match cell {
            CellRef::Bool(v) => KeyRef::Bool(v),
            CellRef::Integer(v) => KeyRef::Integer(v),
            CellRef::String(v) => KeyRef::String(v),
            CellRef::Ip(v) => KeyRef::Ip(v),
            CellRef::Double(_) => bail!("a double cannot be used as a lookup key"),
            CellRef::Null => bail!("a null cannot be used as a lookup key"),
        })
    }

    fn hash_into(self, h: &mut Xxh3) {
        // every variant is tagged so `Integer(1)` and `String("1")` differ, and
        // strings are length-prefixed so ("a","bc") and ("ab","c") differ
        match self {
            KeyRef::Bool(v) => h.update(&[1, v as u8]),
            KeyRef::Integer(v) => {
                h.update(&[2]);
                h.update(&v.to_le_bytes());
            }
            KeyRef::String(v) => {
                h.update(&[3]);
                h.update(&(v.len() as u64).to_le_bytes());
                h.update(v.as_bytes());
            }
            KeyRef::Ip(IpAddr::V4(v)) => {
                h.update(&[4]);
                h.update(&v.octets());
            }
            KeyRef::Ip(IpAddr::V6(v)) => {
                h.update(&[5]);
                h.update(&v.octets());
            }
        }
    }
}

/// An owned key part, for mutable tables and stored partition tuples.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyOwned {
    Bool(bool),
    Integer(i64),
    String(SmolStr),
    Ip(IpAddr),
}

impl KeyOwned {
    pub fn as_ref(&self) -> KeyRef<'_> {
        match self {
            KeyOwned::Bool(v) => KeyRef::Bool(*v),
            KeyOwned::Integer(v) => KeyRef::Integer(*v),
            KeyOwned::String(v) => KeyRef::String(v.as_str()),
            KeyOwned::Ip(v) => KeyRef::Ip(*v),
        }
    }
}

/// The single hash used by both index construction and probing.
pub fn hash_key<'a>(parts: impl IntoIterator<Item = KeyRef<'a>>) -> u64 {
    let mut h = Xxh3::new();
    for part in parts {
        part.hash_into(&mut h);
    }
    h.digest()
}

/// Compares a stored owned key against a borrowed probe, without allocating.
pub fn keys_eq(stored: &[KeyOwned], probe: &[KeyRef<'_>]) -> bool {
    stored.len() == probe.len() && stored.iter().zip(probe).all(|(s, p)| s.as_ref() == *p)
}

pub fn hash_key_owned(parts: &[KeyOwned]) -> u64 {
    hash_key(parts.iter().map(KeyOwned::as_ref))
}

/// The point being looked up in a range-keyed table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangePoint {
    Integer(i64),
    Ip(IpAddr),
}

impl RangePoint {
    /// The point a cell names in a `range_type` index, or `None` when the cell
    /// is the wrong shape. Shared by the builder and both processors, so a
    /// table cannot be built against one rule and probed against another.
    #[inline]
    pub fn from_cell(cell: CellRef<'_>, range_type: RangeType) -> Option<Self> {
        match (range_type, cell) {
            (RangeType::Integer, CellRef::Integer(v)) => Some(RangePoint::Integer(v)),
            (RangeType::Ip, CellRef::Ip(v)) => Some(RangePoint::Ip(v)),
            _ => None,
        }
    }
}

/// A probe against a table.
#[derive(Debug, Clone, Copy)]
pub enum Lookup<'a> {
    Hash(&'a [KeyRef<'a>]),
    Range {
        /// must match the table's `partition_columns`, in order; empty when the
        /// table is unpartitioned
        partition: &'a [KeyRef<'a>],
        point: RangePoint,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(parts: &[KeyRef<'_>]) -> u64 {
        hash_key(parts.iter().copied())
    }

    #[test]
    fn composite_key_parts_do_not_run_together() {
        assert_ne!(
            h(&[KeyRef::String("a"), KeyRef::String("bc")]),
            h(&[KeyRef::String("ab"), KeyRef::String("c")]),
        );
    }

    #[test]
    fn key_variants_are_tagged() {
        assert_ne!(h(&[KeyRef::Integer(1)]), h(&[KeyRef::String("1")]));
        assert_ne!(h(&[KeyRef::Bool(true)]), h(&[KeyRef::Integer(1)]));
    }

    #[test]
    fn same_key_hashes_the_same_across_owned_and_borrowed() {
        let owned = vec![KeyOwned::String("hello".into()), KeyOwned::Integer(7)];
        assert_eq!(
            hash_key_owned(&owned),
            h(&[KeyRef::String("hello"), KeyRef::Integer(7)])
        );
    }

    #[test]
    fn strings_borrow_rather_than_allocate() {
        let source = String::from("jakarta");
        let cell = CellRef::String(&source);
        match cell.to_value() {
            Value::String(z) => assert_eq!(z.as_str().as_ptr(), source.as_ptr()),
            other => panic!(
                "expected a string, got {other:?}",
                other = other.into_owned()
            ),
        }
    }

    #[test]
    fn ip_values_stringify() {
        let cell = CellRef::Ip("10.0.0.1".parse().unwrap());
        match cell.to_value() {
            Value::String(z) => assert_eq!(z.as_str(), "10.0.0.1"),
            _ => panic!("expected a string"),
        }
    }

    #[test]
    fn an_empty_field_is_not_null_by_itself() {
        // the source maps raw values to null before parsing, so `parse` only
        // has to answer "is this a valid value of this type"
        assert_eq!(
            CellRef::parse("", ValueType::String).unwrap(),
            CellRef::String("")
        );
        for ty in [
            ValueType::Bool,
            ValueType::Double,
            ValueType::Integer,
            ValueType::Ip,
        ] {
            assert!(CellRef::parse("", ty).is_err(), "{ty:?}");
        }
    }

    #[test]
    fn parses_each_type() {
        assert_eq!(
            CellRef::parse("42", ValueType::Integer).unwrap(),
            CellRef::Integer(42)
        );
        assert_eq!(
            CellRef::parse("1.5", ValueType::Double).unwrap(),
            CellRef::Double(1.5)
        );
        assert_eq!(
            CellRef::parse("YES", ValueType::Bool).unwrap(),
            CellRef::Bool(true)
        );
        assert_eq!(
            CellRef::parse("0", ValueType::Bool).unwrap(),
            CellRef::Bool(false)
        );
        assert!(CellRef::parse("maybe", ValueType::Bool).is_err());
        assert!(CellRef::parse("x", ValueType::Integer).is_err());
    }

    #[test]
    fn ip_parsing_accepts_text_and_integers_and_canonicalizes() {
        assert_eq!(
            parse_ip("1.2.3.4").unwrap(),
            "1.2.3.4".parse::<IpAddr>().unwrap()
        );
        // bare u32 is the v4 numeric form
        assert_eq!(
            parse_ip("16909060").unwrap(),
            IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))
        );
        // ipv4-mapped v6 folds onto v4 so build and probe agree
        assert_eq!(
            parse_ip("::ffff:1.2.3.4").unwrap(),
            IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))
        );
        assert_eq!(
            parse_ip("2001:db8::1").unwrap(),
            "2001:db8::1".parse::<IpAddr>().unwrap()
        );
        assert!(parse_ip("not-an-ip").is_err());
    }

    #[test]
    fn doubles_are_rejected_as_keys() {
        assert!(KeyRef::try_from_cell(CellRef::Double(1.0)).is_err());
        assert!(KeyRef::try_from_cell(CellRef::Integer(1)).is_ok());
    }
}
