// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use anyhow::{Result, bail};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use strum_macros::{Display, EnumDiscriminants, EnumString};
use utoipa::ToSchema;
use uuid::Uuid;

/// Whether a validation pass may reject a table because a source file is not on
/// this node's filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceFiles {
    MustExist,
    MayBeMissing,
}

/// Deserialized through [`LookupTableSpec`], which lets a `bgp` table leave
/// out its fixed key. Serialization always writes the key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(try_from = "LookupTableSpec")]
pub struct LookupTable {
    pub id: Uuid,
    pub name: String,
    pub source: Source,
    /// Required except for a `bgp` source, whose key is fixed (see
    /// [`Bgp::key`]); a `bgp` table may still spell it out, but only exactly.
    #[schema(required = false)]
    pub key: Key,
}

/// The wire shape of [`LookupTable`], with the key optional.
///
/// Carries `deny_unknown_fields` on its behalf: `try_from` hands the whole
/// deserialization to this type, so the attribute is only enforced here.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupTableSpec {
    id: Uuid,
    name: String,
    source: Source,
    #[serde(default)]
    key: Option<Key>,
}

impl TryFrom<LookupTableSpec> for LookupTable {
    type Error = String;

    fn try_from(spec: LookupTableSpec) -> Result<Self, Self::Error> {
        let key = match (spec.key, &spec.source) {
            (Some(key), _) => key,
            (None, Source::Bgp(_)) => Bgp::key(),
            (None, _) => return Err("key is required".into()),
        };
        Ok(LookupTable {
            id: spec.id,
            name: spec.name,
            source: spec.source,
            key,
        })
    }
}

impl LookupTable {
    /// Held in memory and written at runtime, rather than loaded from files.
    pub fn is_mutable(&self) -> bool {
        matches!(self.source, Source::Processor(_) | Source::Bgp(_))
    }

    /// Writable by `attribute_collect`. A `bgp` table is mutable but written
    /// only by its listener, which relies on seeing every write in order.
    pub fn is_processor_writable(&self) -> bool {
        matches!(self.source, Source::Processor(_))
    }

    /// Full check, for the local config file: a missing source path is a typo
    /// the operator should see at startup, before the process serves anything.
    pub fn validate(&mut self) -> Result<()> {
        self.check(SourceFiles::MustExist)
    }

    /// Everything except whether the source files are present, for a table
    /// read from the database.
    ///
    /// The row is cluster-wide but the paths are node-local, so a file missing
    /// *here* is an operational condition for the loader to report through
    /// [`crate::cache::lookup_table::status::TableHealth`]. Every other check
    /// still runs: a key column that is not a column is wrong on every node.
    pub fn validate_structure(&mut self) -> Result<()> {
        self.check(SourceFiles::MayBeMissing)
    }

    fn check(&mut self, files: SourceFiles) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("name must not be empty");
        }
        // destructured so the key check can borrow `source` immutably while
        // `key` is validated
        let LookupTable { source, key, .. } = self;
        source.validate(files)?;
        // Checked here as well as on parse: a database row stores the key
        // beside the source and never goes through `LookupTableSpec`.
        if matches!(source, Source::Bgp(_)) && *key != Bgp::key() {
            bail!("a bgp table's key is fixed; leave key out");
        }
        key.validate(source)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema, EnumDiscriminants)]
#[serde(rename_all = "snake_case", tag = "type")]
#[strum_discriminants(
    name(KeyKind),
    derive(Serialize, Deserialize, EnumString, Display, ToSchema),
    vis(pub),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
pub enum Key {
    Hash(Hash),
    /// longest-prefix match on an IP prefix (CIDR) column
    Prefix(Prefix),
}

impl Key {
    /// The variant payload without the `type` tag — the shape the `key` column
    /// stores, its tag living in `key_type` beside it. Unlike
    /// [`crate::config::sink::Config::to_json_value`] this enum is internally
    /// tagged, so the reader merges the tag back rather than nesting under it.
    pub fn to_json_value(&self) -> Result<serde_json::Value> {
        match self {
            Self::Hash(hash) => Ok(serde_json::to_value(hash)?),
            Self::Prefix(prefix) => Ok(serde_json::to_value(prefix)?),
        }
    }

    fn validate(&self, source: &Source) -> Result<()> {
        // `None` when the column set is only known once the file is read; those
        // references are re-checked against the resolved schema at load time.
        let columns = source.known_columns();
        let known = columns
            .as_ref()
            .map(|c| c.iter().map(String::as_str).collect::<FxHashSet<_>>());
        let check = |role: &str, col: &str| -> Result<()> {
            if col.trim().is_empty() {
                bail!("{role} must not be empty");
            }
            if let Some(known) = &known
                && !known.contains(col)
            {
                bail!("{role} {col:?} is not a column of this table");
            }
            Ok(())
        };

        match self {
            Key::Hash(hash) => {
                if hash.key_columns.is_empty() {
                    bail!("key.key_columns must not be empty");
                }
                let mut seen = FxHashSet::default();
                for col in &hash.key_columns {
                    check("key.key_columns entry", col)?;
                    if !seen.insert(col.as_str()) {
                        bail!("key.key_columns contains {col:?} more than once");
                    }
                    if source.value_type_of(col) == ValueType::Double {
                        bail!("key column {col:?} is a double; doubles cannot be hash keys");
                    }
                }
            }
            Key::Prefix(prefix) => {
                let pcol = &prefix.prefix_column;
                check("key.prefix_column", pcol)?;
                let mut seen = FxHashSet::default();
                for col in &prefix.partition_columns {
                    check("key.partition_columns entry", col)?;
                    if !seen.insert(col.as_str()) {
                        bail!("key.partition_columns contains {col:?} more than once");
                    }
                    if col == pcol {
                        bail!("key.partition_columns must not contain the prefix column {col:?}");
                    }
                    if source.value_type_of(col) == ValueType::Double {
                        bail!("partition column {col:?} is a double; doubles cannot be keys");
                    }
                }
                // The prefix column is consumed by the index and never
                // materialized, so a table made only of it returns nothing.
                // `ColumnLayout::build` rejects it too, but from inside
                // `LookupTable::replace`, which is all-or-nothing — one such
                // table would stop the node initializing at all. Caught here it
                // is one skipped row, or a named startup error locally.
                if let Some(columns) = &columns
                    && columns.iter().all(|c| c == pcol)
                {
                    bail!(
                        "a prefix table needs at least one column besides key.prefix_column, \
                         which is consumed by the index and never stored"
                    );
                }
                // The prefix column is CIDR text, parsed by the index itself.
                if let Some(declared) = source.declared_value_type_of(pcol)
                    && declared != ValueType::String
                {
                    bail!(
                        "prefix column {pcol:?} is declared {declared:?}, but it holds CIDR \
                         text and must be String"
                    );
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Bool,
    Double,
    Integer,
    #[default]
    String,
    Ip,
}

/// A longest-prefix-match key: a probe address finds the narrowest stored
/// prefix containing it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Prefix {
    /// the column holding the prefix as CIDR text (`10.1.0.0/16`,
    /// `2001:db8::/32`); a bare address is a host route. A prefix with host
    /// bits set (`10.0.0.1/8`) is rejected. Consumed by the index, not stored
    /// as a value column.
    pub prefix_column: String,
    /// columns that split the table into independent prefix sets (a VRF, a BGP
    /// peer); a probe must name one
    #[serde(default)]
    pub partition_columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Hash {
    pub key_columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema, EnumDiscriminants)]
#[serde(rename_all = "snake_case", tag = "type")]
#[strum_discriminants(
    name(SourceKind),
    derive(Serialize, Deserialize, EnumString, Display, ToSchema),
    vis(pub),
    strum(serialize_all = "snake_case"),
    serde(rename_all = "snake_case")
)]
pub enum Source {
    File(File),
    // Database,
    /// actively updated from the pipeline by a processor
    Processor(Processor),
    /// routes written by a BGP/BMP listener, with a fixed schema and key
    Bgp(Bgp),
}

impl Source {
    /// The variant payload alone, without the `type` tag: the shape the `source`
    /// column stores, whose tag lives in `type` beside it. See
    /// [`Key::to_json_value`] for why the tag is split out at all.
    pub fn to_json_value(&self) -> Result<serde_json::Value> {
        match self {
            Self::File(file) => Ok(serde_json::to_value(file)?),
            Self::Processor(processor) => Ok(serde_json::to_value(processor)?),
            Self::Bgp(bgp) => Ok(serde_json::to_value(bgp)?),
        }
    }

    fn validate(&mut self, files: SourceFiles) -> Result<()> {
        match self {
            Source::File(file) => file.validate(files),
            Source::Processor(processor) => processor.validate(),
            Source::Bgp(bgp) => bgp.validate(),
        }
    }

    /// The column set, when it is knowable without reading the source. `None`
    /// means the schema comes from the file's own header row.
    pub fn known_columns(&self) -> Option<Vec<String>> {
        match self {
            Source::File(File {
                format: Format::Csv(csv),
                ..
            }) => (!csv.headers.is_empty()).then(|| csv.headers.clone()),
            Source::Processor(processor) => {
                Some(processor.columns.iter().map(|c| c.name.clone()).collect())
            }
            Source::Bgp(_) => Some(
                bgp::COLUMNS
                    .iter()
                    .map(|(name, _)| name.to_string())
                    .collect(),
            ),
        }
    }

    pub fn declared_value_type_of(&self, column: &str) -> Option<ValueType> {
        match self {
            Source::File(File {
                format: Format::Csv(csv),
                ..
            }) => csv.type_cast.get(column).copied(),
            Source::Processor(processor) => processor
                .columns
                .iter()
                .find(|c| c.name == column)
                .map(|c| c.value_type),
            Source::Bgp(_) => bgp::COLUMNS
                .iter()
                .find(|(name, _)| *name == column)
                .map(|(_, ty)| *ty),
        }
    }

    /// Falls back to the default cast.
    pub fn value_type_of(&self, column: &str) -> ValueType {
        self.declared_value_type_of(column).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub format: Format,
    /// files are concatenated, they should have the same schema
    pub paths: Vec<String>,
    #[serde(default)]
    pub compression: Compression,
    #[serde(default)]
    pub location: Location,
    /// how often the source files are checked for changes. the table is only
    /// rebuilt when a file's modification time or size actually changed.
    #[serde(
        default = "default_reload_interval",
        with = "crate::common::serde::duration_seconds"
    )]
    #[schema(value_type = u64)]
    pub reload_interval: Duration,
}

fn default_reload_interval() -> Duration {
    Duration::from_secs(60)
}

impl File {
    fn validate(&mut self, files: SourceFiles) -> Result<()> {
        if self.paths.is_empty() {
            bail!("source.paths must not be empty");
        }
        if self.reload_interval.is_zero() {
            bail!("source.reload_interval must be greater than zero");
        }
        if files == SourceFiles::MustExist {
            match self.location {
                Location::FileSystem => {
                    for path in &self.paths {
                        if !Path::new(path).exists() {
                            bail!("source.paths entry {path:?} does not exist");
                        }
                    }
                }
            }
        }
        self.format.validate()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Location {
    #[default]
    FileSystem,
    // AmazonS3,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Compression {
    #[default]
    None,
    Gzip,
    Zstd,
    Lz4,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Format {
    Csv(Csv),
    // Parquet,
}

impl Format {
    fn validate(&mut self) -> Result<()> {
        match self {
            Format::Csv(csv) => csv.validate(),
        }
    }
}

fn default_delimiter() -> char {
    ','
}

fn default_quote() -> char {
    '"'
}

fn default_null_values() -> Vec<String> {
    vec![String::new()]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Csv {
    #[serde(default)]
    pub type_cast: FxHashMap<String, ValueType>,
    #[serde(default = "default_delimiter")]
    pub delimiter: char,
    #[serde(default = "default_quote")]
    pub quote: char,
    /// escape character inside quoted fields, for dumps that write `\"` rather
    /// than the RFC 4180 `""`. Doubling stays understood either way, so this
    /// only adds an alternative; unset means doubling is the only escape.
    #[serde(default)]
    pub escape: Option<char>,
    /// field values read as null rather than as data. Defaults to `[""]`, so an
    /// empty field is null; set it to `[]` to make an empty field an empty
    /// string, or list sentinels such as `["", "\\N"]` for a MySQL-style dump.
    #[serde(default = "default_null_values")]
    pub null_values: Vec<String>,
    /// overrides the header row when set, and is required when `has_headers` is
    /// false
    #[serde(default)]
    pub headers: Vec<String>,
    /// whether the source files carry a header row to skip.
    ///
    /// `Option`, and required whenever `headers` is set: that overrides the
    /// column *names* and says nothing about a header line, so defaulting to
    /// `false` silently fed the header row in as data. The `csv` crate's own
    /// default is the opposite, so neither guess is the obvious one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_headers: Option<bool>,
}

impl Csv {
    /// Absent means no.
    pub fn has_headers(&self) -> bool {
        self.has_headers.unwrap_or(false)
    }

    fn validate(&mut self) -> Result<()> {
        if !self.delimiter.is_ascii() {
            bail!("format.delimiter must be an ASCII character");
        }
        if !self.quote.is_ascii() {
            bail!("format.quote must be an ASCII character");
        }
        if self.delimiter == self.quote {
            bail!("format.delimiter and format.quote must differ");
        }
        if let Some(escape) = self.escape {
            if !escape.is_ascii() {
                bail!("format.escape must be an ASCII character");
            }
            if escape == self.delimiter || escape == self.quote {
                bail!("format.escape must differ from format.delimiter and format.quote");
            }
        }
        if !self.has_headers() && self.headers.is_empty() {
            bail!("format.headers is required when format.has_headers is false");
        }
        if !self.headers.is_empty() && self.has_headers.is_none() {
            bail!(
                "format.has_headers must be set explicitly when format.headers is given: \
                 the names come from format.headers either way, and this says whether the \
                 file's own first row is a header to skip or a row of data"
            );
        }
        validate_column_names("format.headers", &self.headers)?;
        if !self.headers.is_empty() {
            self.validate_type_cast_columns(self.headers.iter().map(String::as_str))?;
        }
        Ok(())
    }

    /// Also called after a loader resolves headers from the source file.
    pub(crate) fn validate_type_cast_columns<'a>(
        &self,
        columns: impl Iterator<Item = &'a str>,
    ) -> Result<()> {
        if self.type_cast.is_empty() {
            return Ok(());
        }
        let known: FxHashSet<_> = columns.collect();
        for column in self.type_cast.keys() {
            if !known.contains(column.as_str()) {
                bail!("format.type_cast refers to unknown column {column:?}");
            }
        }
        Ok(())
    }
}

/// A table fed by the pipeline. It has no file to infer a schema from, so the
/// columns are declared up front.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Processor {
    pub columns: Vec<ColumnSpec>,
    /// rows untouched for this long are dropped. zero disables expiry.
    #[serde(
        default = "default_ttl",
        with = "crate::common::serde::duration_seconds"
    )]
    #[schema(value_type = u64)]
    pub ttl: Duration,
    /// hard cap on stored rows. once reached, writes that would introduce a new
    /// row are rejected; existing rows can still be updated.
    #[serde(default = "default_max_rows")]
    pub max_rows: usize,
}

fn default_ttl() -> Duration {
    Duration::from_secs(3600)
}

fn default_max_rows() -> usize {
    1_000_000
}

impl Processor {
    fn validate(&mut self) -> Result<()> {
        if self.columns.is_empty() {
            bail!("source.columns must not be empty");
        }
        if self.max_rows == 0 {
            bail!("source.max_rows must be greater than zero");
        }
        let names: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        validate_column_names("source.columns", &names)
    }
}

/// A route table written by one BGP/BMP listener.
///
/// The columns and the key are fixed ([`bgp::COLUMNS`], [`Bgp::key`]): the
/// listener is the only writer, and it knows one row shape. Which routes it
/// keeps (VRFs, RIB views) is the listener's config, not the table's.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Bgp {
    /// the listener that writes this table
    pub listener: Uuid,
    /// hard cap on stored routes, across every router and VRF. once reached,
    /// new prefixes are rejected; known ones are still updated.
    #[serde(default = "default_bgp_max_rows")]
    pub max_rows: usize,
}

/// A full IPv4 + IPv6 table is about 1.2M routes, so this holds three routers'
/// worth, or many more partial tables.
fn default_bgp_max_rows() -> usize {
    4_000_000
}

impl Bgp {
    /// Prefix-keyed on [`bgp::PREFIX`], partitioned by router and VRF.
    pub fn key() -> Key {
        Key::Prefix(Prefix {
            prefix_column: bgp::PREFIX.to_string(),
            partition_columns: vec![bgp::ROUTER.to_string(), bgp::VRF.to_string()],
        })
    }

    fn validate(&mut self) -> Result<()> {
        if self.max_rows == 0 {
            bail!("source.max_rows must be greater than zero");
        }
        // that `listener` names a bgp listener is checked against the whole
        // config: `LocalFileConfig::validate`
        Ok(())
    }
}

/// The fixed schema of a [`Bgp`] table.
pub mod bgp {
    use super::ValueType;

    /// the router the route came from: the BMP router or the BGP peer
    pub const ROUTER: &str = "router";
    /// `global` for the default table (route distinguisher 0), otherwise the
    /// RD as `ASN:n` or `IP:n`
    pub const VRF: &str = "vrf";
    /// the route's prefix; consumed by the index, returned as CIDR text
    pub const PREFIX: &str = "prefix";
    pub const NEXT_HOP: &str = "next_hop";
    /// the last AS of the path
    pub const ORIGIN_AS: &str = "origin_as";
    /// the first AS of the path: the neighbour the route came through
    pub const PEER_AS: &str = "peer_as";
    /// space separated; an AS set is `{a,b}`
    pub const AS_PATH: &str = "as_path";
    /// space separated `a:b`
    pub const COMMUNITIES: &str = "communities";
    /// space separated `a:b:c`
    pub const LARGE_COMMUNITIES: &str = "large_communities";
    pub const LOCAL_PREF: &str = "local_pref";
    pub const MED: &str = "med";
    /// the ADD-PATH path id (RFC 7911) of the path the row shows; null when
    /// the router does not send path ids
    pub const PATH_ID: &str = "path_id";

    /// The value [`VRF`] holds for the default table.
    pub const GLOBAL_VRF: &str = "global";

    /// Every column in table order, the prefix included.
    pub const COLUMNS: [(&str, ValueType); 12] = [
        (ROUTER, ValueType::Ip),
        (VRF, ValueType::String),
        (PREFIX, ValueType::String),
        (NEXT_HOP, ValueType::Ip),
        (ORIGIN_AS, ValueType::Integer),
        (PEER_AS, ValueType::Integer),
        (AS_PATH, ValueType::String),
        (COMMUNITIES, ValueType::String),
        (LARGE_COMMUNITIES, ValueType::String),
        (LOCAL_PREF, ValueType::Integer),
        (MED, ValueType::Integer),
        (PATH_ID, ValueType::Integer),
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ColumnSpec {
    pub name: String,
    #[serde(default)]
    pub value_type: ValueType,
}

fn validate_column_names(field: &str, names: &[String]) -> Result<()> {
    let mut seen = FxHashSet::default();
    for name in names {
        if name.trim().is_empty() {
            bail!("{field} must not contain an empty column name");
        }
        if !seen.insert(name.as_str()) {
            bail!("{field} contains {name:?} more than once");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct Fixture {
        _dir: tempfile::TempDir,
        path: String,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("table.csv");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"host,site\na,jkt\n").unwrap();
        Fixture {
            path: path.display().to_string(),
            _dir: dir,
        }
    }

    fn csv(headers: &[&str], has_headers: bool) -> Csv {
        Csv {
            type_cast: FxHashMap::default(),
            delimiter: ',',
            quote: '"',
            escape: None,
            null_values: vec![String::new()],
            headers: headers.iter().map(|s| s.to_string()).collect(),
            has_headers: Some(has_headers),
        }
    }

    fn table(fixture: &Fixture, csv: Csv, key: Key) -> LookupTable {
        LookupTable {
            id: Uuid::now_v7(),
            name: "hosts".into(),
            source: Source::File(File {
                format: Format::Csv(csv),
                paths: vec![fixture.path.clone()],
                compression: Compression::None,
                location: Location::FileSystem,
                reload_interval: default_reload_interval(),
            }),
            key,
        }
    }

    fn hash(columns: &[&str]) -> Key {
        Key::Hash(Hash {
            key_columns: columns.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn prefix(partition: &[&str], column: &str) -> Key {
        Key::Prefix(Prefix {
            prefix_column: column.into(),
            partition_columns: partition.iter().map(|s| s.to_string()).collect(),
        })
    }

    #[test]
    fn a_valid_table_passes() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "site"], true), hash(&["host"]));
        t.validate().unwrap();
    }

    /// The prefix column is consumed by the index, so a table made only of it
    /// has no column left to return. `Table::build` rejects it, but for a processor
    /// table that happens inside the all-or-nothing `LookupTable::replace`, so
    /// letting it through here would stop a backend node initializing at all.
    /// Both validation passes must catch it, since only the structural one runs
    /// against a row read from the database.
    #[test]
    fn a_prefix_table_of_nothing_but_its_prefix_is_rejected() {
        let bounds_only = || LookupTable {
            id: Uuid::now_v7(),
            name: "prefixes".into(),
            source: Source::Processor(Processor {
                columns: vec![ColumnSpec {
                    name: "network".into(),
                    value_type: ValueType::String,
                }],
                ttl: default_ttl(),
                max_rows: default_max_rows(),
            }),
            key: prefix(&[], "network"),
        };
        for (pass, mut t) in [
            ("validate", bounds_only()),
            ("validate_structure", bounds_only()),
        ] {
            let err = match pass {
                "validate" => t.validate().unwrap_err(),
                _ => t.validate_structure().unwrap_err(),
            }
            .to_string();
            assert!(err.contains("at least one column besides"), "{pass}: {err}");
        }

        // one value column beside the prefix is enough, and it must still build
        let mut ok = bounds_only();
        let Source::Processor(processor) = &mut ok.source else {
            unreachable!()
        };
        processor.columns.push(ColumnSpec {
            name: "country".into(),
            value_type: ValueType::String,
        });
        ok.validate().unwrap();
        crate::cache::lookup_table::LookupTable::new()
            .insert(&ok)
            .expect("a table that validates must also build");
    }

    /// `headers` renames the columns; it says nothing about whether the file
    /// also opens with a header row. Defaulting that to `false` silently fed
    /// the header line in as data, and the `csv` crate's own default is the
    /// opposite, so there is no obvious guess to make — the operator has to say.
    #[test]
    fn has_headers_must_be_explicit_when_headers_is_given() {
        let f = fixture();
        let mut ambiguous = table(&f, csv(&["host", "site"], true), hash(&["host"]));
        let Source::File(File {
            format: Format::Csv(cfg),
            ..
        }) = &mut ambiguous.source
        else {
            unreachable!("the fixture is file-backed")
        };
        cfg.has_headers = None;
        let err = ambiguous.validate().unwrap_err().to_string();
        assert!(err.contains("has_headers must be set explicitly"), "{err}");

        // either answer is fine, as long as it is given
        for explicit in [true, false] {
            let mut t = table(&f, csv(&["host", "site"], explicit), hash(&["host"]));
            t.validate().unwrap();
        }
        // and omitting it entirely is still fine when the file names its own
        let mut from_file = table(&f, csv(&[], true), hash(&["host"]));
        from_file.validate().unwrap();
    }

    #[test]
    fn column_references_are_checked_when_the_columns_are_known() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "site"], true), hash(&["nope"]));
        let err = t.validate().unwrap_err().to_string();
        assert!(err.contains("not a column"), "{err}");

        // ...and deferred to load time when they are not
        let mut t = table(&f, csv(&[], true), hash(&["anything"]));
        t.validate().unwrap();
    }

    #[test]
    fn a_missing_source_file_is_rejected() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "site"], true), hash(&["host"]));
        if let Source::File(file) = &mut t.source {
            file.paths = vec!["/definitely/not/here.csv".into()];
        }
        let err = t.validate().unwrap_err().to_string();
        assert!(err.contains("does not exist"), "{err}");
    }

    #[test]
    fn an_empty_path_list_is_rejected() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "site"], true), hash(&["host"]));
        if let Source::File(file) = &mut t.source {
            file.paths = vec![];
        }
        assert!(t.validate().is_err());
    }

    #[test]
    fn headers_are_required_for_a_headerless_file() {
        let f = fixture();
        let mut t = table(&f, csv(&[], false), hash(&["host"]));
        let err = t.validate().unwrap_err().to_string();
        assert!(err.contains("required when"), "{err}");

        let mut t = table(&f, csv(&["host", "site"], false), hash(&["host"]));
        t.validate().unwrap();
    }

    #[test]
    fn duplicate_and_empty_header_names_are_rejected() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "host"], true), hash(&["host"]));
        assert!(
            t.validate()
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );

        let mut t = table(&f, csv(&["host", " "], true), hash(&["host"]));
        assert!(t.validate().unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn the_delimiter_and_quote_must_be_distinct_ascii() {
        let f = fixture();
        let mut c = csv(&["host", "site"], true);
        c.delimiter = '"';
        let mut t = table(&f, c, hash(&["host"]));
        assert!(
            t.validate()
                .unwrap_err()
                .to_string()
                .contains("must differ")
        );

        let mut c = csv(&["host", "site"], true);
        c.delimiter = '£';
        let mut t = table(&f, c, hash(&["host"]));
        assert!(t.validate().unwrap_err().to_string().contains("ASCII"));
    }

    #[test]
    fn the_escape_must_be_ascii_and_distinct() {
        let f = fixture();
        for collide in ['"', ','] {
            let mut c = csv(&["host", "site"], true);
            c.escape = Some(collide);
            let mut t = table(&f, c, hash(&["host"]));
            assert!(
                t.validate()
                    .unwrap_err()
                    .to_string()
                    .contains("format.escape must differ"),
                "{collide:?}"
            );
        }

        let mut c = csv(&["host", "site"], true);
        c.escape = Some('£');
        let mut t = table(&f, c, hash(&["host"]));
        assert!(t.validate().unwrap_err().to_string().contains("ASCII"));

        // and a plausible one passes
        let mut c = csv(&["host", "site"], true);
        c.escape = Some('\\');
        let mut t = table(&f, c, hash(&["host"]));
        t.validate().unwrap();
    }

    #[test]
    fn type_cast_must_refer_to_real_columns() {
        let f = fixture();
        let mut c = csv(&["host", "site"], true);
        c.type_cast.insert("ghost".into(), ValueType::Integer);
        let mut t = table(&f, c, hash(&["host"]));
        assert!(
            t.validate()
                .unwrap_err()
                .to_string()
                .contains("unknown column")
        );
    }

    #[test]
    fn a_double_column_cannot_be_a_key() {
        let f = fixture();
        let mut c = csv(&["host", "site"], true);
        c.type_cast.insert("host".into(), ValueType::Double);
        let mut t = table(&f, c, hash(&["host"]));
        let err = t.validate().unwrap_err().to_string();
        assert!(err.contains("doubles cannot be hash keys"), "{err}");
    }

    #[test]
    fn an_empty_key_column_list_is_rejected() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "site"], true), hash(&[]));
        assert!(t.validate().is_err());
    }

    #[test]
    fn duplicate_key_columns_are_rejected() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "site"], true), hash(&["host", "host"]));
        assert!(
            t.validate()
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
    }

    #[test]
    fn the_prefix_column_stays_out_of_the_partition() {
        let f = fixture();
        let c = || csv(&["network", "vrf", "label"], true);

        let mut t = table(&f, c(), prefix(&["network"], "network"));
        let err = t.validate().unwrap_err().to_string();
        assert!(err.contains("must not contain the prefix column"), "{err}");

        let mut t = table(&f, c(), prefix(&["vrf", "vrf"], "network"));
        let err = t.validate().unwrap_err().to_string();
        assert!(err.contains("more than once"), "{err}");

        let mut t = table(&f, c(), prefix(&["nope"], "network"));
        let err = t.validate().unwrap_err().to_string();
        assert!(err.contains("not a column"), "{err}");

        let mut t = table(&f, c(), prefix(&[], "nope"));
        let err = t.validate().unwrap_err().to_string();
        assert!(err.contains("key.prefix_column"), "{err}");

        let mut cc = c();
        cc.type_cast.insert("vrf".into(), ValueType::Double);
        let mut t = table(&f, cc, prefix(&["vrf"], "network"));
        let err = t.validate().unwrap_err().to_string();
        assert!(err.contains("double"), "{err}");

        let mut t = table(&f, c(), prefix(&["vrf"], "network"));
        t.validate().unwrap();
    }

    #[test]
    fn the_prefix_column_must_be_a_string() {
        let f = fixture();
        for ty in [ValueType::Ip, ValueType::Integer] {
            let mut c = csv(&["network", "label"], true);
            c.type_cast.insert("network".into(), ty);
            let mut t = table(&f, c, prefix(&[], "network"));
            let err = t.validate().unwrap_err().to_string();
            assert!(err.contains("must be String"), "{ty:?}: {err}");
        }

        // and an agreeing cast is fine
        let mut c = csv(&["network", "label"], true);
        c.type_cast.insert("network".into(), ValueType::String);
        let mut t = table(&f, c, prefix(&[], "network"));
        t.validate().unwrap();
    }

    #[test]
    fn a_processor_table_needs_unique_named_columns_and_a_cap() {
        let mut t = LookupTable {
            id: Uuid::now_v7(),
            name: "live".into(),
            source: Source::Processor(Processor {
                columns: vec![],
                ttl: default_ttl(),
                max_rows: default_max_rows(),
            }),
            key: hash(&["host"]),
        };
        assert!(
            t.validate()
                .unwrap_err()
                .to_string()
                .contains("must not be empty")
        );

        let column = |name: &str| ColumnSpec {
            name: name.into(),
            value_type: ValueType::String,
        };
        if let Source::Processor(p) = &mut t.source {
            p.columns = vec![column("host"), column("host")];
        }
        assert!(
            t.validate()
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );

        if let Source::Processor(p) = &mut t.source {
            p.columns = vec![column("host"), column("site")];
            p.max_rows = 0;
        }
        assert!(t.validate().unwrap_err().to_string().contains("max_rows"));

        if let Source::Processor(p) = &mut t.source {
            p.max_rows = 10;
        }
        t.validate().unwrap();

        // key columns still have to exist
        t.key = hash(&["nope"]);
        assert!(t.validate().is_err());
    }

    #[test]
    fn an_empty_name_is_rejected() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "site"], true), hash(&["host"]));
        t.name = "  ".into();
        assert!(t.validate().is_err());
    }

    #[test]
    fn yaml_round_trips_with_defaults() {
        let f = fixture();
        let yaml = format!(
            "id: 0192f8b0-0000-7000-8000-000000000001\n\
             name: geoip\n\
             source:\n\
             \x20 type: file\n\
             \x20 location:\n\
             \x20   type: file_system\n\
             \x20 paths: ['{}']\n\
             \x20 format:\n\
             \x20   type: csv\n\
             \x20   has_headers: true\n\
             key:\n\
             \x20 type: hash\n\
             \x20 key_columns: [host]\n",
            f.path
        );
        let mut parsed: LookupTable = serde_yaml_ng::from_str(&yaml).unwrap();
        parsed.validate().unwrap();
        let Source::File(file) = &parsed.source else {
            panic!("expected a file source");
        };
        assert_eq!(file.reload_interval, Duration::from_secs(60));
        assert_eq!(file.compression, Compression::None);
        assert!(!parsed.is_mutable());
    }

    #[test]
    fn a_processor_source_round_trips() {
        let yaml = "id: 0192f8b0-0000-7000-8000-000000000002\n\
                    name: live\n\
                    source:\n\
                    \x20 type: processor\n\
                    \x20 ttl: 120\n\
                    \x20 columns:\n\
                    \x20   - name: host\n\
                    \x20   - name: asn\n\
                    \x20     value_type: integer\n\
                    key:\n\
                    \x20 type: hash\n\
                    \x20 key_columns: [host]\n";
        let mut parsed: LookupTable = serde_yaml_ng::from_str(yaml).unwrap();
        parsed.validate().unwrap();
        assert!(parsed.is_mutable());
        let Source::Processor(p) = &parsed.source else {
            panic!("expected a processor source");
        };
        assert_eq!(p.ttl, Duration::from_secs(120));
        assert_eq!(p.max_rows, default_max_rows());
        assert_eq!(p.columns[0].value_type, ValueType::String);
        assert_eq!(p.columns[1].value_type, ValueType::Integer);
    }

    #[test]
    fn a_bgp_source_takes_its_fixed_key() {
        let yaml = "id: 0192f8b0-0000-7000-8000-000000000003\n\
                    name: routes\n\
                    source:\n\
                    \x20 type: bgp\n\
                    \x20 listener: 0192f8b0-0000-7000-8000-0000000000aa\n";
        let mut parsed: LookupTable = serde_yaml_ng::from_str(yaml).unwrap();
        parsed.validate().unwrap();
        assert_eq!(parsed.key, Bgp::key());
        assert!(parsed.is_mutable());
        assert!(!parsed.is_processor_writable());
        let Source::Bgp(bgp) = &parsed.source else {
            panic!("expected a bgp source");
        };
        assert_eq!(bgp.max_rows, default_bgp_max_rows());
        assert_eq!(
            parsed.source.known_columns().unwrap(),
            bgp::COLUMNS.map(|(name, _)| name.to_string())
        );
        assert_eq!(
            parsed.source.value_type_of(bgp::ROUTER),
            ValueType::Ip,
            "the fixed types, not the string default"
        );

        // serialized with its key, which parses back to the same table
        let json = serde_json::to_value(&parsed).unwrap();
        assert!(json.get("key").is_some());
        let back: LookupTable = serde_json::from_value(json).unwrap();
        assert_eq!(back, parsed);
    }

    #[test]
    fn a_bgp_key_may_only_be_spelled_out_exactly() {
        let with_key = |key: &str| {
            format!(
                "id: 0192f8b0-0000-7000-8000-000000000004\n\
                 name: routes\n\
                 source: {{type: bgp, listener: 0192f8b0-0000-7000-8000-0000000000aa}}\n\
                 key: {key}\n"
            )
        };
        let mut exact: LookupTable = serde_yaml_ng::from_str(&with_key(
            "{type: prefix, prefix_column: prefix, partition_columns: [router, vrf]}",
        ))
        .unwrap();
        exact.validate().unwrap();

        let mut other: LookupTable = serde_yaml_ng::from_str(&with_key(
            "{type: prefix, prefix_column: prefix, partition_columns: [router]}",
        ))
        .unwrap();
        let err = other.validate().unwrap_err().to_string();
        assert!(err.contains("fixed"), "{err}");
    }

    #[test]
    fn only_a_bgp_source_may_leave_the_key_out() {
        let yaml = "id: 0192f8b0-0000-7000-8000-000000000005\n\
                    name: live\n\
                    source:\n\
                    \x20 type: processor\n\
                    \x20 columns: [{name: host}]\n";
        let err = serde_yaml_ng::from_str::<LookupTable>(yaml)
            .unwrap_err()
            .to_string();
        assert!(err.contains("key is required"), "{err}");

        // and the spec still rejects a misplaced field
        let yaml = "id: 0192f8b0-0000-7000-8000-000000000006\n\
                    name: routes\n\
                    source: {type: bgp, listener: 0192f8b0-0000-7000-8000-0000000000aa}\n\
                    ttl: 60\n";
        let err = serde_yaml_ng::from_str::<LookupTable>(yaml)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field"), "{err}");
    }

    #[test]
    fn a_bgp_table_needs_a_cap() {
        let mut t = LookupTable {
            id: Uuid::now_v7(),
            name: "routes".into(),
            source: Source::Bgp(Bgp {
                listener: Uuid::now_v7(),
                max_rows: 0,
            }),
            key: Bgp::key(),
        };
        assert!(t.validate().unwrap_err().to_string().contains("max_rows"));
        if let Source::Bgp(bgp) = &mut t.source {
            bgp.max_rows = 1;
        }
        t.validate().unwrap();
    }

    /// The local config file names paths on the machine that reads it, so a path
    /// that is not there is a typo and the operator should be told at startup.
    #[test]
    fn validate_rejects_a_source_file_that_is_not_on_this_node() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "site"], true), hash(&["host"]));
        if let Source::File(file) = &mut t.source {
            file.paths = vec!["/nonexistent/definitely/not/here.csv".into()];
        }
        let err = format!("{:#}", t.validate().unwrap_err());
        assert!(err.contains("does not exist"), "{err}");
    }

    /// A database row is cluster-wide but its paths are node-local, so the file
    /// being absent *here* must not make this node answer as though the table
    /// were never configured. It is registered and the loader reports the error.
    #[test]
    fn validate_structure_accepts_a_source_file_that_is_not_on_this_node() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "site"], true), hash(&["host"]));
        if let Source::File(file) = &mut t.source {
            file.paths = vec!["/nonexistent/definitely/not/here.csv".into()];
        }
        t.validate_structure().unwrap();
    }

    /// The split drops *only* the filesystem check. A key column that is not a
    /// column is wrong on every node and no retry will fix it, so it must still
    /// be rejected on the database path.
    #[test]
    fn validate_structure_still_rejects_a_key_column_that_is_not_declared() {
        let f = fixture();
        let mut t = table(&f, csv(&["host", "site"], true), hash(&["ghost"]));
        let err = format!("{:#}", t.validate_structure().unwrap_err());
        assert!(err.contains("not a column"), "{err}");

        // and the rest of the structural checks too
        let mut t = table(&f, csv(&["host", "site"], true), hash(&["host"]));
        if let Source::File(file) = &mut t.source {
            file.paths.clear();
        }
        let err = format!("{:#}", t.validate_structure().unwrap_err());
        assert!(err.contains("must not be empty"), "{err}");
    }

    /// `Source` and `Key` are stored as a `type`/`key_type` text column plus a
    /// body blob, and the reader merges the tag back in with
    /// `jsonb_build_object('type', type) || source`. Assemble exactly what that
    /// SQL produces and prove it deserializes back to the same value.
    ///
    /// This is the one place a disagreement between `#[serde(rename_all)]` on the
    /// enum and `strum(serialize_all)` on its discriminants would surface, and it
    /// catches it without a database.
    #[test]
    fn the_tag_column_and_the_body_column_recombine_into_the_config() {
        /// What the jsonb merge does: the one-key tag object, then the body.
        fn merge(tag: &str, body: serde_json::Value) -> serde_json::Value {
            let mut merged = serde_json::Map::new();
            merged.insert("type".into(), serde_json::Value::String(tag.into()));
            let serde_json::Value::Object(body) = body else {
                panic!("a variant payload must serialize to a json object");
            };
            merged.extend(body);
            serde_json::Value::Object(merged)
        }

        let f = fixture();
        let sources = [
            table(&f, csv(&["host", "site"], true), hash(&["host"])).source,
            Source::Processor(Processor {
                columns: vec![ColumnSpec {
                    name: "host".into(),
                    value_type: ValueType::String,
                }],
                ttl: Duration::from_secs(30),
                max_rows: 7,
            }),
            Source::Bgp(Bgp {
                listener: Uuid::now_v7(),
                max_rows: 9,
            }),
        ];
        for source in sources {
            // the tag column, written the way an insert will write it
            let tag = SourceKind::from(&source).to_string();
            let embedded = merge(&tag, source.to_json_value().unwrap());
            let back: Source = serde_json::from_value(embedded).unwrap();
            assert_eq!(back, source, "source did not survive the tag/body split");
        }

        let keys = [hash(&["host", "site"]), prefix(&["site"], "network")];
        for key in keys {
            let tag = KeyKind::from(&key).to_string();
            let embedded = merge(&tag, key.to_json_value().unwrap());
            let back: Key = serde_json::from_value(embedded).unwrap();
            assert_eq!(back, key, "key did not survive the tag/body split");
        }
    }
}

/// Keeps the documented examples in `config-full.yaml` honest: the block there
/// is commented out, so nothing else would catch it drifting from these types.
#[cfg(test)]
mod config_full_yaml {
    use super::*;

    fn uncommented_lookup_block() -> String {
        let text =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/config-full.yaml"))
                .expect("config-full.yaml");
        let start = text
            .find("# lookup_tables:")
            .expect("the lookup_tables example block");
        let rest = &text[start..];
        let end = rest
            .find("\n# listeners —")
            .map(|i| rest[..i].rfind("\n# ───").unwrap_or(i))
            .unwrap_or(rest.len());
        rest[..end]
            .lines()
            .map(|line| {
                line.strip_prefix("# ")
                    .unwrap_or(line.strip_prefix('#').unwrap_or(line))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[derive(serde::Deserialize)]
    struct Block {
        lookup_tables: Vec<LookupTable>,
    }

    #[test]
    fn the_documented_examples_still_deserialize() {
        let yaml = uncommented_lookup_block();
        let block: Block = serde_yaml_ng::from_str(&yaml).unwrap_or_else(|e| {
            panic!("config-full.yaml lookup_tables example is stale: {e}\n{yaml}")
        });
        assert!(
            block.lookup_tables.len() >= 4,
            "expected an example of each shape, got {}",
            block.lookup_tables.len()
        );

        let by_name = |name: &str| {
            block
                .lookup_tables
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("no example named {name}"))
        };

        // the geoip example exercises multi-path, compression and casts
        let geo = by_name("geoip_city");
        let Source::File(file) = &geo.source else {
            panic!("expected a file source")
        };
        assert_eq!(file.paths.len(), 2);
        assert_eq!(file.compression, Compression::Gzip);
        assert_eq!(file.reload_interval, Duration::from_secs(3600));
        assert!(matches!(&geo.key, Key::Prefix(p) if p.prefix_column == "network"));
        let Format::Csv(geo_csv) = &file.format;
        assert_eq!(geo_csv.escape, Some('\\'));
        assert_eq!(geo_csv.null_values, ["", "\\N"]);

        assert!(matches!(
            &by_name("vrf_subnets").key,
            Key::Prefix(p) if p.prefix_column == "subnet" && p.partition_columns == ["vrf"]
        ));
        let owners = by_name("host_owners");
        assert!(matches!(
            &owners.key,
            Key::Hash(h) if h.key_columns == ["tenant", "hostname"]
        ));
        let Source::File(owners_file) = &owners.source else {
            panic!("expected a file source")
        };
        let Format::Csv(owners_csv) = &owners_file.format;
        assert!(
            owners_csv.null_values.is_empty(),
            "the example documents the empty-string case"
        );

        let live = by_name("interfaces");
        assert!(live.is_mutable());
        let Source::Processor(p) = &live.source else {
            panic!("expected a processor source")
        };
        assert_eq!(p.columns.len(), 5);
        assert_eq!(p.max_rows, 1_000_000);

        let mut routes = by_name("routes").clone();
        routes.validate().unwrap();
        assert_eq!(
            routes.key,
            Bgp::key(),
            "the fixed key, though none is given"
        );
        assert!(!routes.is_processor_writable());
        let Source::Bgp(bgp) = &routes.source else {
            panic!("expected a bgp source")
        };
        assert_eq!(bgp.max_rows, default_bgp_max_rows());
    }
}
