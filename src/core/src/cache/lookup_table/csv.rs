// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::cell::CellRef;
use super::immutable::{ImmutableBuilder, RowSource};
use crate::config::lookup_table::{Csv, ValueType};
use anyhow::{Context, Result, anyhow, bail};
use csv::{ByteRecord, Reader, ReaderBuilder};
use smol_str::SmolStr;
use std::io::Read;

/// A CSV record, parsed per column on demand.
pub struct TextRow<'a> {
    record: &'a ByteRecord,
    /// raw values that mean null rather than data, from [`Csv::null_values`]
    nulls: &'a [String],
}

impl<'a> TextRow<'a> {
    pub fn new(record: &'a ByteRecord, nulls: &'a [String]) -> Self {
        Self { record, nulls }
    }
}

impl RowSource for TextRow<'_> {
    fn len(&self) -> usize {
        self.record.len()
    }

    fn cell(&self, index: usize, ty: ValueType) -> Result<CellRef<'_>> {
        let raw = self
            .record
            .get(index)
            .ok_or_else(|| anyhow!("no field at index {index}"))?;
        let text = std::str::from_utf8(raw).map_err(|e| anyhow!("invalid utf-8: {e}"))?;
        // the default list is just `[""]`, so this is one length comparison
        if self.nulls.iter().any(|n| n == text) {
            return Ok(CellRef::Null);
        }
        CellRef::parse(text, ty)
    }
}

pub fn reader<R: Read>(inner: R, cfg: &Csv) -> Result<Reader<R>> {
    let delimiter = ascii(cfg.delimiter, "delimiter")?;
    let quote = ascii(cfg.quote, "quote")?;
    // `None` leaves RFC 4180 doubling as the only escape, which is the default
    let escape = cfg.escape.map(|c| ascii(c, "escape")).transpose()?;
    Ok(ReaderBuilder::new()
        .delimiter(delimiter)
        .quote(quote)
        .escape(escape)
        .has_headers(cfg.has_headers())
        // a row with the wrong field count comes through as a record and is
        // skipped by the builder's arity check, rather than aborting the load
        .flexible(true)
        .from_reader(inner))
}

fn ascii(c: char, what: &str) -> Result<u8> {
    u8::try_from(u32::from(c)).map_err(|_| anyhow!("csv {what} must be an ASCII character"))
}

/// Resolves the column names for one file.
///
/// Returns the effective columns and, when the file carries a header row, that
/// row verbatim so it can be compared across concatenated files.
pub fn headers<R: Read>(
    rdr: &mut Reader<R>,
    cfg: &Csv,
) -> Result<(Vec<SmolStr>, Option<Vec<String>>)> {
    let file_headers = if cfg.has_headers() {
        Some(
            rdr.byte_headers()
                .context("reading the csv header row")?
                .iter()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };

    // configured headers win, which is how a file with unhelpful column names
    // gets usable ones
    let columns: Vec<SmolStr> = if !cfg.headers.is_empty() {
        cfg.headers.iter().map(SmolStr::new).collect()
    } else {
        file_headers
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(SmolStr::new)
            .collect()
    };
    if columns.is_empty() {
        bail!("could not determine any columns for this file");
    }
    Ok((columns, file_headers))
}

/// Feeds every remaining record into `builder`. Individual bad rows are counted
/// by the builder; only an unreadable file is an error.
pub fn read_rows<R: Read>(
    rdr: &mut Reader<R>,
    cfg: &Csv,
    builder: &mut ImmutableBuilder,
) -> Result<()> {
    let mut record = ByteRecord::new();
    loop {
        match rdr.read_byte_record(&mut record) {
            Ok(true) => {
                builder.push_row(&TextRow::new(&record, &cfg.null_values));
            }
            Ok(false) => return Ok(()),
            Err(e) => return Err(anyhow!(e)).context("reading a csv record"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::lookup_table::immutable::source_columns;
    use crate::config::lookup_table as cfg;
    use rustc_hash::FxHashMap;
    use std::time::Duration;

    fn csv_cfg(has_headers: bool, headers: &[&str]) -> Csv {
        let has_headers = Some(has_headers);
        Csv {
            type_cast: FxHashMap::default(),
            delimiter: ',',
            quote: '"',
            escape: None,
            null_values: vec![String::new()],
            headers: headers.iter().map(|s| s.to_string()).collect(),
            has_headers,
        }
    }

    fn hash_key(columns: &[&str]) -> cfg::Key {
        cfg::Key::Hash(cfg::Hash {
            key_columns: columns.iter().map(|s| s.to_string()).collect(),
        })
    }

    /// Mirrors what the loader does for a single file.
    fn load(
        text: &str,
        cfg: &Csv,
        key: &cfg::Key,
    ) -> Result<super::super::immutable::ImmutableData> {
        let mut rdr = reader(text.as_bytes(), cfg)?;
        let (columns, _) = headers(&mut rdr, cfg)?;
        let source = source_columns(&columns, key, |n| {
            cfg.type_cast.get(n).copied().unwrap_or_default()
        });
        let mut builder = ImmutableBuilder::new(&source, key)?;
        read_rows(&mut rdr, cfg, &mut builder)?;
        builder.finish(Duration::ZERO)
    }

    #[test]
    fn takes_columns_from_the_header_row() {
        let key = hash_key(&["host"]);
        let data = load("host,site\na,jkt\nb,bdg\n", &csv_cfg(true, &[]), &key).unwrap();
        assert_eq!(data.schema().names(), &["host", "site"]);
        assert_eq!(data.rows(), 2);
    }

    #[test]
    fn configured_headers_override_the_file() {
        let key = hash_key(&["h"]);
        let cfg = csv_cfg(true, &["h", "s"]);
        let data = load("host,site\na,jkt\n", &cfg, &key).unwrap();
        assert_eq!(data.schema().names(), &["h", "s"]);
        assert_eq!(data.rows(), 1, "the header row is still consumed");
    }

    #[test]
    fn headerless_files_use_the_configured_names() {
        let key = hash_key(&["host"]);
        let data = load("a,jkt\nb,bdg\n", &csv_cfg(false, &["host", "site"]), &key).unwrap();
        assert_eq!(data.schema().names(), &["host", "site"]);
        assert_eq!(data.rows(), 2);
    }

    #[test]
    fn an_empty_field_becomes_null() {
        use crate::cache::lookup_table::cell::{KeyRef, Lookup};
        let key = hash_key(&["host"]);
        let data = load("host,site\na,\n", &csv_cfg(true, &[]), &key).unwrap();
        let site = data.schema().index_of("site").unwrap();
        let row = data.lookup(Lookup::Hash(&[KeyRef::String("a")])).unwrap();
        assert_eq!(data.cell(row, site), CellRef::Null);
    }

    #[test]
    fn a_configured_escape_is_honoured_alongside_doubling() {
        use crate::cache::lookup_table::cell::{KeyRef, Lookup};
        let key = hash_key(&["host"]);
        let text = concat!(
            "host,note\n",
            r#"a,"x\"y""#,
            "\n",
            r#"b,"say ""hi""""#,
            "\n"
        );

        // without an escape, `\"` closes the quote and the rest is junk
        let data = load(text, &csv_cfg(true, &[]), &key).unwrap();
        let note = data.schema().index_of("note").unwrap();
        let at = |data: &super::super::immutable::ImmutableData, h: &str| {
            data.lookup(Lookup::Hash(&[KeyRef::String(h)]))
                .map(|r| data.cell(r, note).as_str().unwrap_or_default().to_string())
        };
        assert_eq!(at(&data, "a").as_deref(), Some(r#"x\y""#));
        assert_eq!(
            at(&data, "b").as_deref(),
            Some(r#"say "hi""#),
            "doubling is understood by default"
        );

        // with one, both styles work
        let mut cfg = csv_cfg(true, &[]);
        cfg.escape = Some('\\');
        let data = load(text, &cfg, &key).unwrap();
        assert_eq!(at(&data, "a").as_deref(), Some(r#"x"y"#));
        assert_eq!(at(&data, "b").as_deref(), Some(r#"say "hi""#));
    }

    #[test]
    fn null_values_are_configurable() {
        use crate::cache::lookup_table::cell::{KeyRef, Lookup};
        let key = hash_key(&["host"]);

        // an empty list makes an empty field an empty string rather than null,
        // which is the only way to express one in a csv
        let mut cfg = csv_cfg(true, &[]);
        cfg.null_values = vec![];
        let data = load("host,site\na,\n", &cfg, &key).unwrap();
        let site = data.schema().index_of("site").unwrap();
        let row = data.lookup(Lookup::Hash(&[KeyRef::String("a")])).unwrap();
        assert_eq!(data.cell(row, site), CellRef::String(""));

        // and a sentinel can stand in for null instead, dump style
        let mut cfg = csv_cfg(true, &[]);
        cfg.null_values = vec!["\\N".into()];
        let data = load("host,site\na,\\N\nb,\n", &cfg, &key).unwrap();
        let site = data.schema().index_of("site").unwrap();
        let at = |h: &str| {
            data.lookup(Lookup::Hash(&[KeyRef::String(h)]))
                .map(|r| data.cell(r, site))
        };
        assert_eq!(at("a"), Some(CellRef::Null));
        assert_eq!(at("b"), Some(CellRef::String("")));
    }

    #[test]
    fn typed_columns_are_cast_and_bad_casts_skip_the_row() {
        let key = hash_key(&["host"]);
        let mut cfg = csv_cfg(true, &[]);
        cfg.type_cast.insert("port".into(), ValueType::Integer);
        let data = load("host,port\na,8080\nb,not-a-number\nc,443\n", &cfg, &key).unwrap();
        assert_eq!(data.rows(), 2);
        assert_eq!(data.stats().skipped.bad_cell, 1);
        assert!(
            data.stats()
                .first_error
                .as_deref()
                .unwrap()
                .contains("not-a-number"),
            "{:?}",
            data.stats().first_error
        );
    }

    #[test]
    fn a_short_or_long_row_is_skipped_not_fatal() {
        let key = hash_key(&["host"]);
        let data = load(
            "host,site\na,jkt\nb\nc,bdg,extra\nd,sby\n",
            &csv_cfg(true, &[]),
            &key,
        )
        .unwrap();
        assert_eq!(data.rows(), 2);
        assert_eq!(data.stats().skipped.arity, 2);
    }

    #[test]
    fn quoting_and_a_custom_delimiter_are_honoured() {
        let key = hash_key(&["host"]);
        let mut cfg = csv_cfg(true, &[]);
        cfg.delimiter = ';';
        cfg.quote = '\'';
        let data = load("host;site\na;'jkt;selatan'\n", &cfg, &key).unwrap();
        use crate::cache::lookup_table::cell::{KeyRef, Lookup};
        let site = data.schema().index_of("site").unwrap();
        let row = data.lookup(Lookup::Hash(&[KeyRef::String("a")])).unwrap();
        assert_eq!(data.cell(row, site), CellRef::String("jkt;selatan"));
    }

    #[test]
    fn invalid_utf8_skips_the_row_rather_than_failing_the_load() {
        let key = hash_key(&["host"]);
        let cfg = csv_cfg(true, &[]);
        let mut bytes = b"host,site\na,jkt\nb,".to_vec();
        bytes.push(0xff);
        bytes.push(b'\n');
        let mut rdr = reader(bytes.as_slice(), &cfg).unwrap();
        let (columns, _) = headers(&mut rdr, &cfg).unwrap();
        let source = source_columns(&columns, &key, |_| ValueType::String);
        let mut builder = ImmutableBuilder::new(&source, &key).unwrap();
        read_rows(&mut rdr, &cfg, &mut builder).unwrap();
        let data = builder.finish(Duration::ZERO).unwrap();
        assert_eq!(data.rows(), 1);
        assert_eq!(data.stats().skipped.bad_cell, 1);
    }

    #[test]
    fn an_ip_range_file_loads_end_to_end() {
        use crate::cache::lookup_table::cell::{Lookup, RangePoint};
        let key = cfg::Key::Range(cfg::Range {
            range_type: cfg::RangeType::Ip,
            partition_columns: vec![],
            start_column: "start".into(),
            start_inclusive: true,
            end_column: "end".into(),
            end_inclusive: true,
        });
        let data = load(
            "start,end,country\n\
             1.0.0.0,1.0.0.255,AU\n\
             1.0.1.0,1.0.3.255,CN\n\
             2001:db8::,2001:db8::ffff,ID\n",
            &csv_cfg(true, &[]),
            &key,
        )
        .unwrap();
        assert_eq!(data.schema().names(), &["country"]);
        let country = data.schema().index_of("country").unwrap();
        let at = |s: &str| {
            data.lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Ip(s.parse().unwrap()),
            })
            .map(|r| data.cell(r, country))
        };
        assert_eq!(at("1.0.0.5"), Some(CellRef::String("AU")));
        assert_eq!(at("1.0.2.9"), Some(CellRef::String("CN")));
        assert_eq!(at("2001:db8::1"), Some(CellRef::String("ID")));
        assert_eq!(at("8.8.8.8"), None);
    }
}
