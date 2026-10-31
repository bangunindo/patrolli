// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Reading immutable tables off disk, and keeping them fresh.

use super::immutable::{ImmutableBuilder, ImmutableData, source_columns};
use super::{ImmutableTable, Meta, csv};
use crate::cache::Cache;
use crate::config::lookup_table as cfg;
use anyhow::{Context, Result, bail};
use smol_str::SmolStr;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

/// How often the reloader wakes up. Each table is then only checked once its
/// own `reload_interval` has elapsed.
const CHECK_TICK: Duration = Duration::from_secs(5);

const READ_BUFFER: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PathStamp {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
}

/// The modification time and size of every source file, used to decide whether
/// a rebuild is worth doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStamp(Box<[PathStamp]>);

impl SourceStamp {
    /// A stamp no real set of files can equal, used to force a reload.
    pub fn never_matches() -> Self {
        Self(Box::new([PathStamp {
            path: PathBuf::new(),
            modified: None,
            len: u64::MAX,
        }]))
    }

    /// A file that cannot be stat'd stamps as absent, so it reloads as soon as
    /// it appears.
    pub fn of(paths: &[String]) -> Self {
        Self(
            paths
                .iter()
                .map(|p| {
                    let path = PathBuf::from(p);
                    let meta = std::fs::metadata(&path).ok();
                    PathStamp {
                        modified: meta.as_ref().and_then(|m| m.modified().ok()),
                        len: meta.as_ref().map_or(0, |m| m.len()),
                        path,
                    }
                })
                .collect(),
        )
    }
}

fn open(path: &Path, compression: cfg::Compression) -> Result<Box<dyn Read + Send>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let buffered = BufReader::with_capacity(READ_BUFFER, file);
    Ok(match compression {
        cfg::Compression::None => Box::new(buffered),
        // MultiGzDecoder, not GzDecoder: the latter silently stops at the first
        // member of a multi-member gzip file
        cfg::Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(buffered)),
        cfg::Compression::Zstd => Box::new(
            zstd::stream::read::Decoder::new(buffered)
                .with_context(|| format!("opening {} as zstd", path.display()))?,
        ),
        cfg::Compression::Lz4 => Box::new(lz4_flex::frame::FrameDecoder::new(buffered)),
    })
}

/// Builds one generation of a table from its source files, concatenated.
///
/// Blocking; call it from `spawn_blocking`.
pub fn load(meta: &Meta) -> Result<ImmutableData> {
    let cfg::Source::File(file) = &meta.config.source else {
        bail!("{} is not file-backed", meta.name);
    };
    let cfg::Format::Csv(csv_cfg) = &file.format;
    let started = Instant::now();

    let mut builder: Option<ImmutableBuilder> = None;
    let mut columns: Option<Vec<SmolStr>> = None;
    let mut header_row: Option<Vec<String>> = None;

    for path in &file.paths {
        let raw = open(Path::new(path), file.compression)?;
        let mut reader = csv::reader(raw, csv_cfg)?;
        let (file_columns, file_headers) = csv::headers(&mut reader, csv_cfg)
            .with_context(|| format!("resolving the schema of {path}"))?;

        match &columns {
            None => columns = Some(file_columns.clone()),
            Some(previous) if *previous != file_columns => {
                bail!(
                    "{path} has columns {file_columns:?} but the first file has {previous:?}; \
                     concatenated files must share a schema"
                );
            }
            Some(_) => {}
        }
        // when files carry their own headers, compare those too: configured
        // headers would otherwise mask a genuinely different file
        match (&header_row, &file_headers) {
            (Some(previous), Some(current)) if previous != current => {
                bail!(
                    "{path} has header row {current:?} but the first file has {previous:?}; \
                     concatenated files must share a schema"
                );
            }
            (None, Some(current)) => header_row = Some(current.clone()),
            _ => {}
        }

        if builder.is_none() {
            csv_cfg
                .validate_type_cast_columns(file_columns.iter().map(SmolStr::as_str))
                .with_context(|| format!("resolving the schema of {path}"))?;
            let source = source_columns(file_columns.as_slice(), &meta.config.key, |name| {
                csv_cfg.type_cast.get(name).copied().unwrap_or_default()
            });
            builder = Some(
                ImmutableBuilder::new(&source, &meta.config.key)
                    .with_context(|| format!("planning the schema of {}", meta.name))?,
            );
        }
        csv::read_rows(&mut reader, csv_cfg, builder.as_mut().expect("just built"))
            .with_context(|| format!("reading {path}"))?;
    }

    let builder = builder.ok_or_else(|| anyhow::anyhow!("no source paths"))?;
    builder.finish(started.elapsed())
}

/// Loads a table and stamps its sources, refusing the result if a file changed
/// while it was being read.
fn load_stamped(
    meta: &Meta,
    read: impl FnOnce(&Meta) -> Result<ImmutableData>,
) -> Result<(ImmutableData, SourceStamp), LoadFailure> {
    let paths = match &meta.config.source {
        cfg::Source::File(file) => file.paths.as_slice(),
        cfg::Source::Processor(_) => &[],
    };
    let before = SourceStamp::of(paths);
    let data = read(meta);
    // Check even when parsing failed: a mid-write parse error is retryable too.
    let after = SourceStamp::of(paths);
    if before != after {
        return Err(LoadFailure {
            stamp: after,
            error: anyhow::anyhow!("lookup table {} sources changed while loading", meta.name),
            retry: true,
        });
    }
    data.map(|data| (data, after)).map_err(|error| LoadFailure {
        stamp: before,
        error,
        retry: false,
    })
}

#[derive(Debug)]
struct LoadFailure {
    stamp: SourceStamp,
    error: anyhow::Error,
    retry: bool,
}

/// Loads one table now, replacing its data on success.
///
/// `checked` is the stamp the caller decided to reload on; it is recorded on
/// failure so a file that stays broken is not re-parsed every tick.
pub async fn reload(table: &Arc<ImmutableTable>, checked: SourceStamp) -> Result<()> {
    let meta = Arc::clone(table.meta());
    let name = meta.name.clone();
    let loaded = tokio::task::spawn_blocking(move || load_stamped(&meta, load))
        .await
        .unwrap_or_else(|error| {
            Err(LoadFailure {
                stamp: checked,
                error: anyhow::anyhow!(error).context(format!("lookup table {name} load task")),
                retry: true,
            })
        });
    finish_reload(table, loaded)
}

fn finish_reload(
    table: &ImmutableTable,
    loaded: Result<(ImmutableData, SourceStamp), LoadFailure>,
) -> Result<()> {
    // An empty source is a legitimate first load — see
    // `a_load_with_no_rows_at_all_is_an_empty_table_not_an_error` — but must
    // never replace rows an earlier load produced: a file caught truncated
    // parses cleanly and would clear `last_error` on its way over the good
    // generation. Demoting it to a load failure keeps the serving rows.
    // `retry: false`, so a source that stays empty is not re-parsed every tick.
    let serving = table.data().rows();
    let loaded = match loaded {
        Ok((data, stamp)) if data.rows() == 0 && serving > 0 => Err(LoadFailure {
            stamp,
            error: anyhow::anyhow!(
                "source yielded no rows; keeping the {serving} row(s) already loaded \
                 (remove or disable the table to serve an empty one)"
            ),
            retry: false,
        }),
        other => other,
    };
    match loaded {
        Ok((data, stamp)) => {
            let stats = data.stats().clone();
            let rows = data.rows();
            let index_entries = data.index_entries();
            let skipped = stats.skipped.total();
            let generation = table.install(data, stamp);
            info!(
                lookup_table = %table.meta().name,
                generation,
                rows_read = stats.rows_read,
                rows_skipped = skipped,
                unique_rows = rows,
                index_entries,
                elapsed_ms = stats.elapsed.as_millis() as u64,
                "loaded lookup table"
            );
            if skipped > 0 {
                warn!(
                    lookup_table = %table.meta().name,
                    rows_skipped = skipped,
                    reasons = %stats.skipped.summary(),
                    first = stats.first_error.as_deref().unwrap_or(""),
                    "some lookup table rows were skipped"
                );
            }
            Ok(())
        }
        Err(failure) => {
            let message = format!("{:#}", failure.error);
            table.note_load_error(failure.stamp, message, failure.retry);
            Err(failure.error)
        }
    }
}

/// Checks every immutable table once and reloads the ones whose files changed.
pub async fn check_once(cache: &Cache) {
    check_once_inner(cache, None).await
}

/// `shutdown`, when given, is polled between tables. A sweep awaits each reload
/// in turn and a reload is a whole source parse, so without this a teardown
/// waits out however many multi-second parses the sweep has left to do.
async fn check_once_inner(cache: &Cache, shutdown: Option<&CancellationToken>) {
    for table in cache.lookup_table.immutable_tables() {
        if shutdown.is_some_and(CancellationToken::is_cancelled) {
            return;
        }
        let Some(file) = table.file() else { continue };
        if !table.due_for_check(file.reload_interval) {
            continue;
        }
        let stamp = SourceStamp::of(&file.paths);
        if !table.note_check(&stamp) {
            trace!(lookup_table = %table.meta().name, "lookup table sources unchanged");
            continue;
        }
        if let Err(e) = reload(&table, stamp).await {
            warn!(
                lookup_table_id = %table.meta().id,
                lookup_table = %table.meta().name,
                error = %format!("{e:#}"),
                loaded = table.is_loaded(),
                "failed to load lookup table, keeping the previous data"
            );
        }
    }
}

/// Ages out mutable rows and sweeps their string pools.
pub fn maintain_mutable(cache: &Cache) {
    for table in cache.lookup_table.mutable_tables() {
        let expired = table.table().expire(super::mutable::now_millis());
        let released = table.table().compact(false);
        table.meta().metrics.note_sweep(expired, released);
        let rejected = table.table().take_rejected_delta();
        if rejected > 0 {
            warn!(
                lookup_table_id = %table.meta().id,
                lookup_table = %table.meta().name,
                rejected,
                max_rows = table.table().max_rows(),
                "mutable lookup table at capacity, new keys rejected"
            );
        }
        if expired > 0 || released > 0 {
            debug!(
                lookup_table = %table.meta().name,
                expired,
                released,
                rows = table.table().len(),
                "swept mutable lookup table"
            );
        }
    }
}

/// Registers the configured tables and loads every file-backed one before the
/// pipeline starts, so no table is briefly empty.
pub async fn init(configs: &[cfg::LookupTable], cache: &Cache) -> Result<()> {
    cache.lookup_table.replace(configs)?;
    for table in cache.lookup_table.immutable_tables() {
        let Some(file) = table.file() else { continue };
        let stamp = SourceStamp::of(&file.paths);
        // Honour the answer, as `check_once` does: `replace` keeps an unchanged
        // table, so on a re-initialize — every node-lock flap runs this again —
        // re-parsing every dump only delays the node going active.
        if !table.note_check(&stamp) {
            continue;
        }
        if let Err(e) = reload(&table, stamp).await {
            // a table that cannot load yet is not fatal: the reloader retries,
            // and lookups against it simply miss
            warn!(
                lookup_table_id = %table.meta().id,
                lookup_table = %table.meta().name,
                error = %format!("{e:#}"),
                "lookup table starts empty"
            );
        }
    }
    Ok(())
}

/// Runs the reload and maintenance loops until `shutdown` fires.
///
/// Two tasks rather than two arms of one `select!`: a reload can take tens of
/// seconds, and the TTL sweep is the only thing that frees capacity — queued
/// behind a reload, a table at its cap would reject keys for the whole parse.
pub fn spawn_reloader(cache: Cache, shutdown: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        let reloads = tokio::spawn(reload_loop(cache.clone(), shutdown.clone()));
        let sweeps = tokio::spawn(sweep_loop(cache, shutdown));
        let (reloads, sweeps) = tokio::join!(reloads, sweeps);
        if let Err(e) = reloads {
            error!(error = %e, "lookup table reload loop failed");
        }
        if let Err(e) = sweeps {
            error!(error = %e, "lookup table sweep loop failed");
        }
    })
}

async fn reload_loop(cache: Cache, shutdown: CancellationToken) {
    let mut tick = tokio::time::interval(CHECK_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tick.tick() => check_once_inner(&cache, Some(&shutdown)).await,
        }
    }
}

async fn sweep_loop(cache: Cache, shutdown: CancellationToken) {
    let mut tick = tokio::time::interval(CHECK_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tick.tick() => {
                // Blocking: `expire` walks every row of every mutable table
                // behind each shard's write lock and `compact` takes them all
                // again — far too much for a runtime worker at the default
                // million-row cap.
                let cache = cache.clone();
                if let Err(e) = tokio::task::spawn_blocking(move || maintain_mutable(&cache)).await {
                    error!(error = %e, "lookup table maintenance task failed, sweeps stopped");
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::lookup_table::cell::{CellRef, KeyRef, Lookup, RangePoint};
    use std::io::Write;
    use uuid::Uuid;

    fn csv_format() -> cfg::Format {
        cfg::Format::Csv(cfg::Csv {
            type_cast: Default::default(),
            delimiter: ',',
            quote: '"',
            escape: None,
            null_values: vec![String::new()],
            headers: vec![],
            has_headers: Some(true),
        })
    }

    fn config(
        name: &str,
        paths: Vec<String>,
        key: cfg::Key,
        compression: cfg::Compression,
    ) -> cfg::LookupTable {
        cfg::LookupTable {
            id: Uuid::now_v7(),
            name: name.to_string(),
            source: cfg::Source::File(cfg::File {
                format: csv_format(),
                paths,
                compression,
                location: cfg::Location::FileSystem,
                // so every `check_once` in a test actually re-stats
                reload_interval: Duration::from_nanos(1),
            }),
            key,
        }
    }

    fn hash_key(columns: &[&str]) -> cfg::Key {
        cfg::Key::Hash(cfg::Hash {
            key_columns: columns.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn write(path: &Path, contents: &str) {
        std::fs::write(path, contents).unwrap();
    }

    #[tokio::test]
    async fn metadata_edits_preserve_loaded_rows_and_in_flight_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");
        let mut conf = config(
            "before",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        let cache = Cache::new();
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        let before = cache.lookup_table.get(&conf.id).unwrap();
        let old_loader = before.as_immutable().unwrap();
        conf.name = "after".into();
        cache.lookup_table.upsert(&conf).unwrap();
        if let cfg::Source::File(file) = &mut conf.source {
            file.reload_interval = Duration::from_secs(3600);
        }
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        let after = cache.lookup_table.get(&conf.id).unwrap();
        assert_eq!(after.name(), "after");
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("jkt"));
        assert_eq!(generation(&cache, &conf.id), 1);
        assert_eq!(after.as_immutable().unwrap().attempts(), 1);
        assert_eq!(before.meta().created_at, after.meta().created_at);
        assert!(Arc::ptr_eq(&before.meta().metrics, &after.meta().metrics));

        // A reload already holding the old metadata still reaches the registry.
        write(&path, "host,site\na,bdg\n");
        reload(old_loader, SourceStamp::never_matches())
            .await
            .unwrap();
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("bdg"));
        assert_eq!(generation(&cache, &conf.id), 2);
    }

    #[tokio::test]
    async fn source_edits_keep_the_last_good_generation_until_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("old.csv");
        let new_path = dir.path().join("new.csv");
        write(&old_path, "host,site\na,jkt\n");
        let mut conf = config(
            "hosts",
            vec![old_path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        let cache = Cache::new();
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        let old_table = cache.lookup_table.get(&conf.id).unwrap();
        let old_reader = old_table.read();
        if let cfg::Source::File(file) = &mut conf.source {
            file.paths = vec![new_path.display().to_string()];
        }
        cache.lookup_table.upsert(&conf).unwrap();
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("jkt"));
        check_once(&cache).await; // Missing replacement source.
        assert_eq!(generation(&cache, &conf.id), 1);
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("jkt"));
        assert!(
            cache
                .lookup_table
                .get(&conf.id)
                .unwrap()
                .as_immutable()
                .unwrap()
                .last_error()
                .is_some()
        );

        write(&new_path, "host,site\na,bdg\n");
        check_once(&cache).await;
        assert_eq!(generation(&cache, &conf.id), 2);
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("bdg"));
        assert_eq!(
            old_reader
                .lookup(Lookup::Hash(&[KeyRef::String("a")]))
                .unwrap()
                .cell(old_reader.column("site").unwrap()),
            CellRef::String("jkt")
        );

        // Completion of a loader for the superseded source cannot roll it back.
        write(&old_path, "host,site\na,stale\n");
        reload(
            old_table.as_immutable().unwrap(),
            SourceStamp::never_matches(),
        )
        .await
        .unwrap();
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("bdg"));
        assert_eq!(generation(&cache, &conf.id), 2);
    }

    #[tokio::test]
    async fn key_edits_reload_even_when_the_file_stamp_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");
        let mut conf = config(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        let cache = Cache::new();
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        conf.key = hash_key(&["site"]);
        cache
            .lookup_table
            .replace(std::slice::from_ref(&conf))
            .unwrap();
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("jkt"));
        check_once(&cache).await;
        assert_eq!(generation(&cache, &conf.id), 2);
        assert_eq!(site(&cache, &conf.id, "a"), None);
        assert_eq!(site(&cache, &conf.id, "jkt").as_deref(), Some("jkt"));
    }

    /// A source that parses to zero rows must not replace rows an earlier load
    /// produced. Before this, a file caught truncated to nothing but its header
    /// parsed cleanly, installed over the good generation and cleared
    /// `last_error` with it, so enrichment stopped with no failure anywhere.
    #[tokio::test]
    async fn an_emptied_source_does_not_replace_a_populated_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\nb,bdg\n");
        let conf = config(
            "emptied",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        let cache = Cache::new();
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        let table = cache.lookup_table.get(&conf.id).unwrap();
        let immutable = table.as_immutable().unwrap();
        let loaded = immutable.data();
        assert_eq!(loaded.rows(), 2);

        // regenerated down to just its header row
        write(&path, "host,site\n");
        check_once(&cache).await;

        assert!(
            Arc::ptr_eq(&loaded, &immutable.data()),
            "the populated generation must still be the one serving"
        );
        assert_eq!(immutable.generation(), 1, "no generation may be installed");
        assert_eq!(
            immutable
                .data()
                .lookup(Lookup::Hash(&[KeyRef::String("a")])),
            Some(0),
            "and it must still answer"
        );
        let error = immutable.last_error().expect("the refusal is recorded");
        assert!(error.contains("no rows"), "{error}");
        assert!(error.contains('2'), "it names what was kept: {error}");
        assert_eq!(table.status().totals.load_failures, 1);

        // refilling the file recovers on the next check
        write(&path, "host,site\na,sby\n");
        check_once(&cache).await;
        assert_eq!(immutable.generation(), 2);
        assert!(immutable.last_error().is_none());
    }

    /// `note_error` records the stamp of a failed read so a source that stays
    /// broken is left alone until it changes. That only works if `note_check`
    /// keys on the stamp: keyed on `generation`, a table that had never
    /// *successfully* loaded looked changed forever and re-parsed the whole
    /// source on every tick — for a large dump with the wrong delimiter, a full
    /// re-read every `reload_interval` for the life of the process.
    #[tokio::test]
    async fn a_source_that_stays_broken_is_not_re_parsed_every_tick() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        // two columns in the header, one field per row: every row is rejected,
        // so the load fails outright and no generation is ever installed
        write(&path, "host,site\na|jkt\nb|bdg\n");
        let conf = config(
            "broken",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        let cache = Cache::new();
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        let table = cache.lookup_table.get(&conf.id).unwrap();
        let immutable = table.as_immutable().unwrap();
        assert_eq!(immutable.generation(), 0, "nothing could be loaded");
        assert_eq!(immutable.attempts(), 1);
        assert!(immutable.last_error().is_some());

        // the fixture re-stats on every tick, so this is purely about the stamp
        for _ in 0..5 {
            check_once(&cache).await;
        }
        assert_eq!(
            immutable.attempts(),
            1,
            "an unchanged bad source is left alone"
        );

        // fixing the file is a change, and is picked up on the next tick
        write(&path, "host,site\na,jkt\n");
        check_once(&cache).await;
        assert_eq!(immutable.attempts(), 2);
        assert_eq!(immutable.generation(), 1);
        assert!(immutable.last_error().is_none());
    }

    /// The other half of the rule: with nothing to protect, an empty source is
    /// still a perfectly good empty table, as `ImmutableBuilder::finish` says.
    #[tokio::test]
    async fn a_first_load_of_an_empty_source_is_an_empty_table_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\n");
        let conf = config(
            "empty-first",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        let cache = Cache::new();
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        let table = cache.lookup_table.get(&conf.id).unwrap();
        let immutable = table.as_immutable().unwrap();
        assert_eq!(immutable.generation(), 1);
        assert_eq!(immutable.data().rows(), 0);
        assert!(immutable.last_error().is_none());
    }

    #[tokio::test]
    async fn unstable_loads_preserve_the_generation_and_retry_next_tick() {
        for parse_error in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("hosts.csv");
            write(&path, "host,site\na,good\n");
            let mut conf = config(
                "unstable",
                vec![path.display().to_string()],
                hash_key(&["host"]),
                cfg::Compression::None,
            );
            let cfg::Source::File(file) = &mut conf.source else {
                unreachable!()
            };
            file.reload_interval = Duration::from_secs(3600);
            let cache = Cache::new();
            init(std::slice::from_ref(&conf), &cache).await.unwrap();
            let table = cache.lookup_table.get(&conf.id).unwrap();
            let immutable = table.as_immutable().unwrap();
            let previous = immutable.data();
            let loaded_at = table.status().loaded_at;
            let attempt = load_stamped(table.meta(), |meta| {
                let result = load(meta);
                // Inside the stamped read, after staging the old contents.
                write(&path, "host,site\na,recovered\nb,another\n");
                if parse_error {
                    bail!("simulated partial-file parse error");
                }
                result
            });
            assert!(finish_reload(immutable, attempt).is_err());
            assert!(Arc::ptr_eq(&previous, &immutable.data()));
            assert_eq!(immutable.generation(), 1);
            assert_eq!(immutable.attempts(), 2);
            assert_eq!(table.status().loaded_at, loaded_at);
            assert!(
                immutable
                    .last_error()
                    .unwrap()
                    .contains("changed while loading")
            );
            assert!(immutable.due_for_check(Duration::from_secs(3600)));
            check_once(&cache).await;
            assert_eq!(immutable.generation(), 2);
            assert_eq!(immutable.attempts(), 3);
            assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("recovered"));
            assert!(immutable.last_error().is_none());
            assert!(!immutable.due_for_check(Duration::from_secs(3600)));
        }
    }

    #[tokio::test]
    async fn inferred_headers_validate_cast_names_again_on_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,good\n");
        let mut conf = config(
            "casts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        let cfg::Source::File(file) = &mut conf.source else {
            unreachable!()
        };
        let cfg::Format::Csv(csv) = &mut file.format;
        csv.type_cast.insert("site".into(), cfg::ValueType::String);
        conf.validate().unwrap();
        let cache = Cache::new();
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        write(&path, "host,renamed\na,changed\n");
        check_once(&cache).await;
        let table = cache.lookup_table.get(&conf.id).unwrap();
        assert_eq!(table.read().generation(), 1);
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("good"));
        assert!(
            table
                .as_immutable()
                .unwrap()
                .last_error()
                .unwrap()
                .contains("unknown column \"site\"")
        );
        let fresh = Cache::new();
        init(std::slice::from_ref(&conf), &fresh).await.unwrap();
        assert_eq!(generation(&fresh, &conf.id), 0);
    }

    fn compress(contents: &str, how: cfg::Compression) -> Vec<u8> {
        match how {
            cfg::Compression::None => contents.as_bytes().to_vec(),
            cfg::Compression::Gzip => {
                let mut e =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(contents.as_bytes()).unwrap();
                e.finish().unwrap()
            }
            cfg::Compression::Zstd => zstd::stream::encode_all(contents.as_bytes(), 3).unwrap(),
            cfg::Compression::Lz4 => {
                let mut e = lz4_flex::frame::FrameEncoder::new(Vec::new());
                e.write_all(contents.as_bytes()).unwrap();
                e.finish().unwrap()
            }
        }
    }

    /// Reads one cell out of a table by key, for terse assertions.
    fn site(cache: &Cache, id: &Uuid, host: &str) -> Option<String> {
        let table = cache.lookup_table.get(id)?;
        let read = table.read();
        let column = read.column("site")?;
        read.lookup(Lookup::Hash(&[KeyRef::String(host)]))
            .map(|row| row.cell(column).as_str().unwrap_or_default().to_string())
    }

    fn generation(cache: &Cache, id: &Uuid) -> u64 {
        cache
            .lookup_table
            .get(id)
            .map_or(0, |t| t.read().generation())
    }

    #[tokio::test]
    async fn init_loads_every_file_backed_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\nb,bdg\n");

        let cache = Cache::new();
        let conf = config(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        init(std::slice::from_ref(&conf), &cache).await.unwrap();

        assert_eq!(generation(&cache, &conf.id), 1);
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("jkt"));
        assert_eq!(site(&cache, &conf.id, "b").as_deref(), Some("bdg"));
        assert_eq!(site(&cache, &conf.id, "c"), None);
    }

    #[tokio::test]
    async fn a_changed_file_is_reloaded_and_bumps_the_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");

        let cache = Cache::new();
        let conf = config(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("jkt"));

        write(&path, "host,site\na,surabaya\nb,medan\n");
        check_once(&cache).await;

        assert_eq!(generation(&cache, &conf.id), 2);
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("surabaya"));
        assert_eq!(site(&cache, &conf.id, "b").as_deref(), Some("medan"));
    }

    #[tokio::test]
    async fn an_unchanged_file_is_not_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");

        let cache = Cache::new();
        let conf = config(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        init(std::slice::from_ref(&conf), &cache).await.unwrap();

        for _ in 0..3 {
            check_once(&cache).await;
        }
        assert_eq!(
            generation(&cache, &conf.id),
            1,
            "re-parsing an unchanged file would be pure waste"
        );
    }

    #[tokio::test]
    async fn lookup_metrics_track_failed_initial_load_and_reload_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\nbad\n");
        let cache = Cache::new();
        let conf = config(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        let snapshot = || super::super::metrics::snapshot(&cache).remove(0);
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        let failed = snapshot();
        assert!(failed.load_error);
        assert_eq!(failed.generation, 0);
        assert_eq!(failed.last_success_timestamp, 0.0);
        assert_eq!(failed.load_failures, 1);

        write(&path, "host,site\na,jkt\nbad\n");
        check_once(&cache).await;
        let loaded = snapshot();
        assert!(!loaded.load_error);
        assert_eq!(loaded.generation, 1);
        assert_eq!(loaded.rows, 1);
        assert_eq!(loaded.generation_rows_skipped.arity, 1);
        assert!(loaded.last_success_timestamp > 0.0);

        std::fs::remove_file(&path).unwrap();
        check_once(&cache).await;
        let failed = snapshot();
        assert!(failed.load_error);
        assert_eq!(failed.generation, loaded.generation);
        assert_eq!(failed.rows, loaded.rows);
        assert_eq!(failed.last_success_timestamp, loaded.last_success_timestamp);
        assert_eq!(failed.load_failures, 2);

        write(&path, "host,site\na,bdg\n");
        check_once(&cache).await;
        let recovered = snapshot();
        assert!(!recovered.load_error);
        assert_eq!(recovered.generation, 2);
        assert!(recovered.last_success_timestamp >= loaded.last_success_timestamp);
        assert_eq!(recovered.generation_rows_skipped.total(), 0);
        assert_eq!(recovered.rows_skipped.arity, 1);
        assert_eq!(recovered.load_failures, 2);
    }

    #[tokio::test]
    async fn a_failing_reload_keeps_the_previously_loaded_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");

        let cache = Cache::new();
        let conf = config(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        init(std::slice::from_ref(&conf), &cache).await.unwrap();

        std::fs::remove_file(&path).unwrap();
        check_once(&cache).await;

        assert_eq!(generation(&cache, &conf.id), 1, "no new generation");
        assert_eq!(
            site(&cache, &conf.id, "a").as_deref(),
            Some("jkt"),
            "the previous data survives"
        );
        let table = cache.lookup_table.get(&conf.id).unwrap();
        assert!(table.as_immutable().unwrap().last_error().is_some());

        // and it recovers once the file comes back
        write(&path, "host,site\na,bdg\n");
        check_once(&cache).await;
        assert_eq!(generation(&cache, &conf.id), 2);
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("bdg"));
        assert!(table.as_immutable().unwrap().last_error().is_none());
    }

    #[tokio::test]
    async fn a_file_where_every_row_is_bad_fails_the_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ranges.csv");
        write(&path, "start,end,label\n1,10,low\n");

        let key = cfg::Key::Range(cfg::Range {
            range_type: cfg::RangeType::Integer,
            partition_columns: vec![],
            start_column: "start".into(),
            start_inclusive: true,
            end_column: "end".into(),
            end_inclusive: true,
        });
        let cache = Cache::new();
        let conf = config(
            "ranges",
            vec![path.display().to_string()],
            key,
            cfg::Compression::None,
        );
        init(std::slice::from_ref(&conf), &cache).await.unwrap();

        let table = cache.lookup_table.get(&conf.id).unwrap();
        let label = table.read().column("label").unwrap();
        let at = |p: i64| {
            table
                .read()
                .lookup(Lookup::Range {
                    partition: &[],
                    point: RangePoint::Integer(p),
                })
                .map(|r| r.cell(label).as_str().unwrap_or_default().to_string())
        };
        assert_eq!(at(5).as_deref(), Some("low"));

        // every bound is unparseable now
        write(&path, "start,end,label\nx,y,low\nz,w,high\n");
        check_once(&cache).await;
        assert_eq!(generation(&cache, &conf.id), 1);
        assert_eq!(at(5).as_deref(), Some("low"), "old generation still serves");
        let error = table.as_immutable().unwrap().last_error().unwrap();
        assert!(error.contains("every row was rejected"), "{error}");
    }

    #[tokio::test]
    async fn a_file_that_stays_broken_is_not_re_parsed_every_tick() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");

        let cache = Cache::new();
        let conf = config(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        let table = cache.lookup_table.get(&conf.id).unwrap();
        let immutable = table.as_immutable().unwrap().clone();
        assert_eq!(immutable.attempts(), 1);

        // break it: every row now has too few fields
        write(&path, "host,site\na\nb\n");
        check_once(&cache).await;
        assert_eq!(immutable.attempts(), 2, "the change was picked up");
        assert!(immutable.last_error().is_some());

        // it stays broken, so re-parsing it on every tick would be pure waste
        for _ in 0..5 {
            check_once(&cache).await;
        }
        assert_eq!(
            immutable.attempts(),
            2,
            "an unchanged broken file is left alone"
        );
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("jkt"));

        // and a fix is still picked up, because fixing it changes the file
        write(&path, "host,site\na,bdg\n");
        check_once(&cache).await;
        assert_eq!(immutable.attempts(), 3);
        assert_eq!(generation(&cache, &conf.id), 2);
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("bdg"));
        assert!(immutable.last_error().is_none());
    }

    #[tokio::test]
    async fn every_compression_codec_round_trips() {
        for how in [
            cfg::Compression::None,
            cfg::Compression::Gzip,
            cfg::Compression::Zstd,
            cfg::Compression::Lz4,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("hosts.csv");
            std::fs::write(&path, compress("host,site\na,jkt\n", how)).unwrap();

            let cache = Cache::new();
            let conf = config(
                "hosts",
                vec![path.display().to_string()],
                hash_key(&["host"]),
                how,
            );
            init(std::slice::from_ref(&conf), &cache).await.unwrap();
            assert_eq!(
                site(&cache, &conf.id, "a").as_deref(),
                Some("jkt"),
                "{how:?}"
            );
        }
    }

    #[tokio::test]
    async fn multi_member_gzip_is_read_to_the_end() {
        // concatenated gzip members are common in vendor dumps, and plain
        // GzDecoder would silently stop after the first
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv.gz");
        let mut bytes = compress("host,site\na,jkt\n", cfg::Compression::Gzip);
        bytes.extend(compress("b,bdg\n", cfg::Compression::Gzip));
        std::fs::write(&path, bytes).unwrap();

        let cache = Cache::new();
        let conf = config(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::Gzip,
        );
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("jkt"));
        assert_eq!(
            site(&cache, &conf.id, "b").as_deref(),
            Some("bdg"),
            "the second gzip member was read"
        );
    }

    #[tokio::test]
    async fn paths_are_concatenated() {
        let dir = tempfile::tempdir().unwrap();
        let v4 = dir.path().join("v4.csv");
        let v6 = dir.path().join("v6.csv");
        write(&v4, "start,end,country\n1.0.0.0,1.0.0.255,AU\n");
        write(&v6, "start,end,country\n2001:db8::,2001:db8::ffff,ID\n");

        let key = cfg::Key::Range(cfg::Range {
            range_type: cfg::RangeType::Ip,
            partition_columns: vec![],
            start_column: "start".into(),
            start_inclusive: true,
            end_column: "end".into(),
            end_inclusive: true,
        });
        let cache = Cache::new();
        let conf = config(
            "geo",
            vec![v4.display().to_string(), v6.display().to_string()],
            key,
            cfg::Compression::None,
        );
        init(std::slice::from_ref(&conf), &cache).await.unwrap();

        let table = cache.lookup_table.get(&conf.id).unwrap();
        let read = table.read();
        let country = read.column("country").unwrap();
        let at = |s: &str| {
            read.lookup(Lookup::Range {
                partition: &[],
                point: RangePoint::Ip(s.parse().unwrap()),
            })
            .map(|r| r.cell(country).as_str().unwrap_or_default().to_string())
        };
        assert_eq!(at("1.0.0.9").as_deref(), Some("AU"));
        assert_eq!(at("2001:db8::1").as_deref(), Some("ID"));
    }

    #[tokio::test]
    async fn concatenated_files_must_share_a_schema() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.csv");
        let b = dir.path().join("b.csv");
        write(&a, "host,site\na,jkt\n");
        write(&b, "host,region\nb,west\n");

        let cache = Cache::new();
        let conf = config(
            "hosts",
            vec![a.display().to_string(), b.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        init(std::slice::from_ref(&conf), &cache).await.unwrap();

        let table = cache.lookup_table.get(&conf.id).unwrap();
        assert_eq!(table.read().generation(), 0, "the load did not succeed");
        let error = table.as_immutable().unwrap().last_error().unwrap();
        assert!(error.contains("must share a schema"), "{error}");
    }

    #[tokio::test]
    async fn a_table_whose_file_is_missing_starts_empty_and_recovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("later.csv");

        let cache = Cache::new();
        let conf = config(
            "later",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        // a table that cannot load yet must not stop the process from starting
        init(std::slice::from_ref(&conf), &cache).await.unwrap();
        assert_eq!(generation(&cache, &conf.id), 0);
        assert!(cache.lookup_table.get(&conf.id).unwrap().read().is_empty());

        write(&path, "host,site\na,jkt\n");
        check_once(&cache).await;
        assert_eq!(generation(&cache, &conf.id), 1);
        assert_eq!(site(&cache, &conf.id, "a").as_deref(), Some("jkt"));
    }

    #[tokio::test]
    async fn the_reload_interval_gates_how_often_files_are_stat_ed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.csv");
        write(&path, "host,site\na,jkt\n");

        let cache = Cache::new();
        let mut conf = config(
            "hosts",
            vec![path.display().to_string()],
            hash_key(&["host"]),
            cfg::Compression::None,
        );
        if let cfg::Source::File(f) = &mut conf.source {
            f.reload_interval = Duration::from_secs(3600);
        }
        init(std::slice::from_ref(&conf), &cache).await.unwrap();

        write(&path, "host,site\na,bdg\n");
        check_once(&cache).await;
        assert_eq!(
            site(&cache, &conf.id, "a").as_deref(),
            Some("jkt"),
            "the interval has not elapsed, so the file is not even stat'd"
        );
    }

    #[tokio::test]
    async fn mutable_maintenance_expires_rows() {
        let cache = Cache::new();
        let conf = cfg::LookupTable {
            id: Uuid::now_v7(),
            name: "live".into(),
            source: cfg::Source::Processor(cfg::Processor {
                columns: vec![
                    cfg::ColumnSpec {
                        name: "host".into(),
                        value_type: cfg::ValueType::String,
                    },
                    cfg::ColumnSpec {
                        name: "site".into(),
                        value_type: cfg::ValueType::String,
                    },
                ],
                // already elapsed by the time maintenance runs
                ttl: Duration::from_nanos(1),
                max_rows: 10,
            }),
            key: hash_key(&["host"]),
        };
        init(std::slice::from_ref(&conf), &cache).await.unwrap();

        let table = cache.lookup_table.get(&conf.id).unwrap();
        let mutable = table.as_mutable().unwrap();
        let _ = mutable
            .table()
            .upsert_hash(&[CellRef::String("a"), CellRef::String("jkt")])
            .unwrap();
        assert_eq!(mutable.table().len(), 1);

        tokio::time::sleep(Duration::from_millis(2)).await;
        maintain_mutable(&cache);
        assert_eq!(mutable.table().len(), 0);
    }

    #[tokio::test]
    async fn the_reloader_stops_on_shutdown() {
        let cache = Cache::new();
        let shutdown = CancellationToken::new();
        let handle = spawn_reloader(cache, shutdown.clone());
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("reloader did not stop")
            .unwrap();
    }
}
