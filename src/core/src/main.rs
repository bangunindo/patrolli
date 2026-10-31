// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use clap::Parser;
use common::mib;
use deadpool_postgres::Runtime;
use patrolli::chan::new as new_channel;
use patrolli::common::internal_metrics::INTERNAL_METRICS;
use patrolli::common::log_export::{self, LOG_EXPORT};
use patrolli::common::shutdown::{Shutdown, join_step};
use patrolli::config::global::{Format, Logger};
use patrolli::{api, cache, chan, cluster, common, config, db, listener, monitor, processor, sink};
use std::fs::OpenOptions;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::fs;
use tokio::signal::unix::{SignalKind, signal};
use tokio_postgres_rustls::MakeRustlsConnect;
use tokio_util::sync::CancellationToken;
use tracing::subscriber::Interest;
use tracing::{Event, Metadata, Subscriber, error, info, span, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::{FilterExt, LevelFilter, Targets};
use tracing_subscriber::fmt::{format::Writer, time::FormatTime};
use tracing_subscriber::layer::{Context as FilterContext, Filter, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_throttle::{Policy, TracingRateLimitLayer};

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// Baked-in jemalloc tuning. Read by jemalloc at init before any allocation.
// `metadata_thp` is init-time only and cannot be set via mallctl later;
// `background_thread` could be toggled at runtime but is set here for symmetry.
//
// `prof:true` initializes heap profiling and `prof_active:false` leaves it idle,
// which is what makes runtime arming possible: `opt.prof` is init-time only, so
// without it here every `prof.active` write would fail with ENOENT. Sampling
// costs nothing until SIGUSR1 arms it (see `install_mem_profile_signals`).
// `lg_prof_sample` is jemalloc's own default, spelled out because it is the knob
// that decides what sampling costs once armed.
//
// The environment still overrides all of this: `MALLOC_CONF` on unprefixed
// targets, `_RJEM_MALLOC_CONF` on the prefixed ones (apple/android/dragonfly) --
// which is also why the exported symbol name below is target-dependent.
#[cfg(feature = "mem-profile")]
const JEMALLOC_CONF: &[u8] =
    b"background_thread:true,metadata_thp:auto,prof:true,prof_active:false,lg_prof_sample:19\0";
#[cfg(not(feature = "mem-profile"))]
const JEMALLOC_CONF: &[u8] = b"background_thread:true,metadata_thp:auto\0";

#[cfg(not(target_env = "msvc"))]
#[allow(non_upper_case_globals)]
#[cfg_attr(
    any(
        target_vendor = "apple",
        target_os = "android",
        target_os = "dragonfly"
    ),
    unsafe(export_name = "_rjem_malloc_conf")
)]
#[cfg_attr(
    not(any(
        target_vendor = "apple",
        target_os = "android",
        target_os = "dragonfly"
    )),
    unsafe(export_name = "malloc_conf")
)]
pub static malloc_conf: &[u8] = JEMALLOC_CONF;

/// Name of the env var jemalloc reads its config from, which is prefixed on the
/// targets where jemalloc itself is built prefixed.
#[cfg(all(feature = "mem-profile", unix))]
const MALLOC_CONF_ENV: &str = if cfg!(any(
    target_vendor = "apple",
    target_os = "android",
    target_os = "dragonfly"
)) {
    "_RJEM_MALLOC_CONF"
} else {
    "MALLOC_CONF"
};

/// jemalloc bails out of its dump path before it ever opens the file on macOS
/// (arming and disarming still work), so point mac devs at Linux rather than at
/// a filesystem problem they do not have.
#[cfg(all(feature = "mem-profile", unix))]
const DUMP_FAILURE_HINT: &str = if cfg!(target_vendor = "apple") {
    "jemalloc cannot write heap dumps on macOS; capture the profile on Linux"
} else {
    "is /tmp writable?"
};

/// Writes a heap profile to `/tmp/patrolli.heap.<pid>.<seq>.prof`, bumping `seq`.
#[cfg(all(feature = "mem-profile", unix))]
fn dump_heap(pid: u32, seq: &mut u32) {
    use std::ffi::CString;
    use tikv_jemalloc_ctl::raw;

    let n = *seq;
    *seq += 1;
    let path = format!("/tmp/patrolli.heap.{pid}.{n}.prof");
    let c_path = match CString::new(path.clone()) {
        Ok(p) => p,
        Err(e) => {
            error!(path, error = %e, "invalid heap dump path");
            return;
        }
    };
    // Safety: writing through the jemalloc `prof.dump` mallctl is the
    // documented way to trigger a heap dump. The C string lives for
    // the duration of the call.
    let res = unsafe { raw::write::<*const std::os::raw::c_char>(b"prof.dump\0", c_path.as_ptr()) };
    match res {
        Ok(()) => info!(path, "wrote heap dump"),
        Err(e) => error!(path, error = %e, hint = DUMP_FAILURE_HINT, "failed to dump heap"),
    }
}

/// Starts or stops sampling on the live process. Returns whether it took.
#[cfg(all(feature = "mem-profile", unix))]
fn set_prof_active(active: bool) -> bool {
    use tikv_jemalloc_ctl::raw;

    // Safety: `prof.active` is a documented rw bool mallctl. jemalloc reads it
    // unlocked on the allocation path, so writing it is the intended way to
    // start and stop sampling without restarting.
    match unsafe { raw::write::<bool>(b"prof.active\0", active) } {
        Ok(()) => true,
        Err(e) => {
            error!(active, error = %e, "failed to set prof.active");
            false
        }
    }
}

/// Installs the SIGUSR1/SIGUSR2 handlers that drive jemalloc heap profiling.
///
/// Profiling is compiled into the default binary and initialized at startup, but
/// boots idle, so a leaking node can be profiled without a rebuild or a restart:
///
/// ```text
/// kill -USR1 <pid>    # arm sampling
///   ... let it take traffic ...
/// kill -USR1 <pid>    # snapshot -> /tmp/patrolli.heap.<pid>.0.prof
///   ... let it take more traffic ...
/// kill -USR1 <pid>    # snapshot -> /tmp/patrolli.heap.<pid>.1.prof
/// kill -USR2 <pid>    # final snapshot, then disarm
/// ```
///
/// Copy the results to your dev machine and read one snapshot, or diff two to
/// isolate what grew in between:
///   jeprof --show_bytes --pdf ./patrolli patrolli.heap.<pid>.0.prof > heap.pdf
///   jeprof --show_bytes --base patrolli.heap.<pid>.0.prof ./patrolli patrolli.heap.<pid>.1.prof
/// Important notes: the binary path MUST MATCH the path used when starting the program.
/// This is important when you're running the program in a remote environment,
/// as the path to the binary may differ.
/// For example, when you run `/home/user/patrolli/patrolli -c config.yaml`
/// Then jeprof should use `jeprof --show_bytes --pdf /home/user/patrolli/patrolli patrolli.heap.*.prof > heap.pdf`
///
/// A `--release` binary resolves to function names only; build with
/// `cargo build --profile profile` (release plus debuginfo) to get file:line.
#[cfg(all(feature = "mem-profile", unix))]
fn install_mem_profile_signals() {
    use tikv_jemalloc_ctl::{profiling, raw};
    use tokio::signal::unix::{SignalKind, signal};

    // `opt.prof` is init-time only, so if it is off the baked-in conf was
    // overridden and neither arming nor dumping can work. Say so once here
    // rather than failing on every signal.
    match profiling::prof::read() {
        Ok(true) => {}
        Ok(false) => {
            // log-style: allow — one-shot operator banner, printed once per process
            warn!(
                "heap profiling is compiled in but was disabled at startup \
                 ({MALLOC_CONF_ENV} overrode prof:true); SIGUSR1/SIGUSR2 will do nothing"
            );
            return;
        }
        Err(e) => {
            error!(error = %e, "failed to read opt.prof, heap profiling unavailable");
            return;
        }
    }

    // jemalloc's configure picks the first backtracer that links and *silently*
    // degrades if the one we asked for is unavailable -- a build image that lost
    // `libunwind-dev` would otherwise fall back unnoticed. So report what we
    // actually got. Safety: these are read-only bool mallctls reflecting
    // compile-time configuration.
    let unwinder = unsafe {
        if raw::read::<bool>(b"config.prof_libunwind\0") == Ok(true) {
            "libunwind"
        } else if raw::read::<bool>(b"config.prof_libgcc\0") == Ok(true) {
            "libgcc"
        } else if raw::read::<bool>(b"config.prof_frameptr\0") == Ok(true) {
            "frame pointer"
        } else {
            // Last resort in configure's list: __builtin_return_address.
            "gcc intrinsics"
        }
    };

    let mut sigusr1 = match signal(SignalKind::user_defined1()) {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "failed to install SIGUSR1 handler for heap profiling");
            return;
        }
    };
    let mut sigusr2 = match signal(SignalKind::user_defined2()) {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "failed to install SIGUSR2 handler for heap profiling");
            return;
        }
    };

    let pid = std::process::id();
    // log-style: allow — one-shot operator banner, printed once per process
    info!(
        "heap profiling ready (jemalloc, {unwinder} backtraces) but idle; \
         SIGUSR1 to pid {pid} arms sampling, each further SIGUSR1 writes \
         /tmp/patrolli.heap.{pid}.<seq>.prof, SIGUSR2 writes a final dump and disarms{}",
        if cfg!(target_vendor = "apple") {
            " -- note: macOS cannot write the dumps, capture profiles on Linux"
        } else {
            ""
        }
    );

    tokio::spawn(async move {
        let mut armed = false;
        let mut seq = 0u32;
        loop {
            // Both `recv`s are cancel-safe, so losing the race cannot drop a signal.
            tokio::select! {
                Some(()) = sigusr1.recv() => {
                    if armed {
                        dump_heap(pid, &mut seq);
                    } else if set_prof_active(true) {
                        armed = true;
                        info!(
                            "heap profiling armed; SIGUSR1 now writes a snapshot, \
                             SIGUSR2 writes a final snapshot and disarms"
                        );
                    }
                }
                Some(()) = sigusr2.recv() => {
                    if !armed {
                        info!("heap profiling was already idle; dumping what is still sampled");
                    }
                    dump_heap(pid, &mut seq);
                    if set_prof_active(false) {
                        armed = false;
                        info!("heap profiling disarmed");
                    }
                }
                else => break,
            }
        }
    });
}

#[cfg(not(all(feature = "mem-profile", unix)))]
#[inline]
fn install_mem_profile_signals() {}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the configuration file
    #[arg(short, long)]
    config: String,
}

/// How long past the shutdown deadline the watchdog waits before it exits
/// the process. Every teardown step is budgeted to end by the deadline; this
/// is for one that is not.
const SHUTDOWN_WATCHDOG_GRACE: Duration = Duration::from_secs(5);

/// Longest the API server waits for open connections on shutdown, cut
/// shorter by the shutdown deadline.
const API_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// The log writer's guard. Statics are never dropped, so whoever ends the
/// process takes and drops it, which flushes what is still buffered: `main`
/// on its way out, [`force_exit`] on an exit that skips destructors.
static LOG_GUARD: Mutex<Option<WorkerGuard>> = Mutex::new(None);

fn flush_logs() {
    drop(LOG_GUARD.lock().unwrap_or_else(|e| e.into_inner()).take());
}

fn force_exit(code: i32) -> ! {
    flush_logs();
    std::process::exit(code)
}

/// Cancels the shutdown token on SIGINT / SIGTERM, then stays to end a
/// shutdown that does not end by itself: a second signal exits at once (the
/// handlers stay installed, so without this it would be swallowed and only
/// SIGKILL would work), and so does the deadline passing by
/// [`SHUTDOWN_WATCHDOG_GRACE`]. A shutdown a task starts by cancelling the
/// token gets the same watchdog, and so does one a panic starts.
fn setup_shutdown(timeout: Duration) -> Shutdown {
    let shutdown = Shutdown::new(timeout);
    common::panic::shutdown_on_panic(shutdown.token());
    let handle = shutdown.clone();
    tokio::spawn(async move {
        let token = handle.token();
        let mut sigint = signal(SignalKind::interrupt()).unwrap();
        let mut sigterm = signal(SignalKind::terminate()).unwrap();
        tokio::select! {
            _ = sigint.recv() => info!(signal = "SIGINT", "shutdown signal received"),
            _ = sigterm.recv() => info!(signal = "SIGTERM", "shutdown signal received"),
            _ = token.cancelled() => {}
        }
        token.cancel();
        let deadline = handle.deadline();
        let code = tokio::select! {
            _ = sigint.recv() => {
                warn!(signal = "SIGINT", "second shutdown signal, exiting now");
                130
            }
            _ = sigterm.recv() => {
                warn!(signal = "SIGTERM", "second shutdown signal, exiting now");
                143
            }
            _ = tokio::time::sleep(deadline.remaining() + SHUTDOWN_WATCHDOG_GRACE) => {
                error!(
                    timeout_ms = handle.timeout().as_millis() as u64,
                    "shutdown did not finish in time, exiting now"
                );
                if common::panic::panicked() {
                    common::panic::EXIT_CODE
                } else {
                    1
                }
            }
        };
        force_exit(code);
    });
    shutdown
}

struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut Writer) -> std::fmt::Result {
        let now = common::now();
        write!(w, "{}", now.to_rfc3339())
    }
}

/// Upper bound on the number of live throttle signatures, and thereby on the
/// memory the throttle layer can hold: each entry retains the rendered text of
/// the event's non-excluded fields, so this count is the only lever that bounds
/// its footprint (see `init_logging`).
///
/// patrolli logs constant messages with low-cardinality fields, so the steady-state
/// signature count is in the hundreds; 2_000 leaves headroom while capping the
/// damage from a large field that slips past the exclusion list. At a generous
/// 8_192 bytes of retained text per entry the layer stays under 16_777_216 bytes,
/// against roughly 600_000_000 bytes if a 28 KiB field filled the crate's
/// 10_000-entry default.
const MAX_LOG_SIGNATURES: usize = 2_000;

pub fn init_logging(config: &Logger) -> WorkerGuard {
    let (non_blocking, guard) = match config.output.as_str() {
        "stdout" => tracing_appender::non_blocking(io::stdout()),
        "stderr" => tracing_appender::non_blocking(io::stderr()),
        "null" => tracing_appender::non_blocking(io::sink()),
        path => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            tracing_appender::non_blocking(file)
        }
    };
    let mut targets = Targets::default()
        .with_target("patrolli", config.level)
        // suppression summaries emitted by the throttle layer below
        .with_target("tracing_throttle::summary", LevelFilter::WARN);
    for module in &config.modules {
        targets = targets.with_target(&module.name, module.level);
    }

    // Rate-limit repeated identical events (same level/target/message/fields)
    // so hot paths can log failures unconditionally. `error`, `request_id` and
    // `stats` are excluded from signatures: their values vary per occurrence
    // without changing what the event means, so including them would give every
    // occurrence a fresh signature and evict the signatures actually being
    // tracked. This is also why messages must be constant literals with the
    // varying parts in fields — an interpolated message defeats the layer.
    //
    // Exclusion is not just about suppression quality, it bounds memory. The
    // layer keeps one entry per live signature and each entry retains the
    // *rendered text* of every non-excluded field (`EventMetadata`, from the
    // crate's default `human-readable` feature). A varying field therefore pins
    // one copy of its text per signature until the cap evicts it. The kafka
    // sink's `stats` field is ~28 KiB of rdkafka `Statistics`, re-rendered
    // every `statistics.interval.ms`: left in the signature it grows RSS by
    // ~28 KiB per entry until the cap is full, which is what took 0.5.5 from
    // 300 MiB to 900 MiB over ~12h in production.
    //
    // `max_signatures` is therefore a memory bound and is set explicitly rather
    // than left to the crate's 10_000 default. Note that the byte-denominated
    // `EvictionStrategy::Memory` cannot do this job: its `estimate_entry_size`
    // is a constant ~480 bytes that never looks at the retained strings, and
    // selecting it *replaces* this count cap instead of adding to it.
    //
    // `logger.disable_throttle` turns the layer off entirely, for when every
    // occurrence of an event must be seen (debugging a burst, or a capture fed
    // to an external aggregator that does its own deduplication). `None` filters
    // nothing, so the cost is that a hot path logging unconditionally is no
    // longer bounded.
    let throttle = NoLevelBound((!config.disable_throttle).then(|| build_throttle(true)));
    // Log export gets its own throttle instance: a clone shares storage, so
    // two layers consulting one instance would charge every event twice.
    // Summaries stay off here so they aren't printed twice.
    let export_throttle = NoLevelBound((!config.disable_throttle).then(|| build_throttle(false)));
    let filter = targets.and(throttle);

    let registry = tracing_subscriber::registry();
    match config.format {
        Format::Json => {
            let layer = tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_timer(LocalTimer)
                .json()
                .with_filter(filter);
            registry
                .with(layer)
                .with(export_layer(export_throttle))
                .init();
        }
        Format::Text => {
            let mut layer = tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_timer(LocalTimer);
            if config.disable_color {
                layer = layer.with_ansi(false);
            }
            let layer = layer.with_filter(filter);
            registry
                .with(layer)
                .with(export_layer(export_throttle))
                .init();
        }
    }
    guard
}

/// Added after the fmt layer so it runs second: stdout is never delayed by
/// export. See `common::log_export` for why its `on_event` never blocks.
fn export_layer<S>(throttle: NoLevelBound<Option<TracingRateLimitLayer>>) -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    log_export::layer(LOG_EXPORT.clone())
        .with_filter(log_export::filter(LOG_EXPORT.clone()).and(throttle))
}

/// Passes every call through to `F` but reports no level bound.
///
/// The throttle limits rates, not levels, yet has no `max_level_hint` (and
/// neither has a `None` `Option` filter), while `and` keeps the *lower* of its
/// two hints, with `None` below every level. One missing hint therefore erases
/// the whole stack's: `LevelFilter::current()` becomes TRACE, `init` copies it
/// into `log`'s max level, and every `log::trace!` from a `log`-based crate
/// walks every layer's filter before being dropped. rdkafka logs one per
/// delivered message. Reporting TRACE lets `and` keep the other side's hint.
struct NoLevelBound<F>(F);

impl<S, F: Filter<S>> Filter<S> for NoLevelBound<F> {
    fn enabled(&self, meta: &Metadata<'_>, cx: &FilterContext<'_, S>) -> bool {
        self.0.enabled(meta, cx)
    }
    fn callsite_enabled(&self, meta: &'static Metadata<'static>) -> Interest {
        self.0.callsite_enabled(meta)
    }
    fn event_enabled(&self, event: &Event<'_>, cx: &FilterContext<'_, S>) -> bool {
        self.0.event_enabled(event, cx)
    }
    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::TRACE)
    }
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, cx: FilterContext<'_, S>) {
        self.0.on_new_span(attrs, id, cx)
    }
    fn on_record(&self, id: &span::Id, values: &span::Record<'_>, cx: FilterContext<'_, S>) {
        self.0.on_record(id, values, cx)
    }
    fn on_enter(&self, id: &span::Id, cx: FilterContext<'_, S>) {
        self.0.on_enter(id, cx)
    }
    fn on_exit(&self, id: &span::Id, cx: FilterContext<'_, S>) {
        self.0.on_exit(id, cx)
    }
    fn on_close(&self, id: span::Id, cx: FilterContext<'_, S>) {
        self.0.on_close(id, cx)
    }
}

fn build_throttle(active_emission: bool) -> TracingRateLimitLayer {
    TracingRateLimitLayer::builder()
        .with_policy(Policy::token_bucket(50.0, 1.0).unwrap())
        .with_excluded_fields(vec![
            "error".to_string(),
            "request_id".to_string(),
            "stats".to_string(),
            "latency_ms".to_string(),
            "user".to_string(),
            "peer".to_string(),
            "batch_id".to_string(),
            "records".to_string(),
            "rows".to_string(),
            "reason".to_string(),
            "row_id".to_string(),
        ])
        .with_max_signatures(MAX_LOG_SIGNATURES)
        .with_summary_interval(Duration::from_secs(60))
        .with_active_emission(active_emission)
        .build()
        .unwrap()
}

fn init_cache(conf: &config::LocalFileConfig, config_cache: &cache::Cache) {
    {
        let mut item_cache = config_cache.item.write();
        for item in &conf.items {
            item_cache.insert(item.clone());
        }
    }
    {
        let mut endpoint_cache = config_cache.endpoint.write();
        for endpoint in &conf.endpoints {
            endpoint_cache.insert(endpoint.clone());
        }
    }
    {
        let mut guard = config_cache.processor.write();
        let processor_cache = guard.as_mut().unwrap();
        for processor in &conf.processors {
            processor_cache.insert_processor(processor.clone());
        }
        for sink in &conf.sinks {
            processor_cache.insert_sink(sink.clone());
        }
        config_cache
            .processor
            .compile(guard)
            .inspect_err(|e| error!(error = %format!("{e:#}"), "processor graph is invalid"))
            .unwrap();
    }
    {
        let mut listener_cache = config_cache.listener.write();
        let endpoint_cache = config_cache.endpoint.read();
        for listener in &conf.listeners {
            listener_cache.insert(listener.clone());
        }
        for item in conf
            .items
            .iter()
            .filter(|item| matches!(item.config, config::item::Config::Listener(_)))
        {
            listener_cache.insert_item(item.clone(), None);
            if let Some(endpoint_id) = item.endpoint_id
                && let Some(endpoint) = endpoint_cache.get(&endpoint_id)
            {
                match endpoint.config.ip() {
                    Some(ip) => listener_cache.insert_ip(endpoint_id, ip),
                    None => warn!(
                        endpoint_id = %endpoint_id,
                        item_id = %item.id,
                        "endpoint address is not an ip, source-ip matching disabled"
                    ),
                }
            }
        }
    }
    {
        let mut host_cache = config_cache.host.write();
        for host in &conf.hosts {
            host_cache.insert(host.clone());
        }
    }
}

async fn init_mib(conf: &config::global::MibConfig) {
    if conf.disabled {
        info!("MIB loading disabled");
        return;
    }
    info!("initializing MIB");
    let dirs: Vec<&str> = conf.dirs.iter().map(|s| s.as_str()).collect();
    match mib::init(&dirs, conf.disable_system_mib).await {
        Ok(()) => info!("MIB initialized"),
        Err(e) => error!(error = %format!("{e:#}"), "failed to initialize MIB"),
    }
}

async fn local_run(conf_content: String) -> Result<()> {
    let mut local_conf: config::LocalFileConfig = serde_yaml_ng::from_str(&conf_content)?;
    drop(conf_content);
    local_conf.validate()?;
    let local_conf = local_conf;
    let shutdown = setup_shutdown(local_conf.global.shutdown_timeout);
    let token = shutdown.token();
    let prometheus_server = local_conf
        .global
        .prometheus
        .as_ref()
        .map(|prometheus| {
            common::prometheus::spawn(
                prometheus,
                Arc::clone(&*INTERNAL_METRICS),
                false,
                token.clone(),
            )
        })
        .transpose()?;
    let config_cache = cache::get_cache();
    init_cache(&local_conf, &config_cache);
    // loads every file-backed lookup table before the pipeline starts, so no
    // table is briefly empty
    cache::lookup_table::loader::init(&local_conf.lookup_tables, &config_cache).await?;
    let lookup_reloader =
        cache::lookup_table::loader::spawn_reloader(config_cache.clone(), token.clone());
    let lookup_reporter = cache::lookup_table::metrics::spawn_metrics_reporter(
        config_cache.clone(),
        local_conf.global.internal_metrics_interval,
        token.clone(),
    );
    let (processor_admin, processor_send) = new_channel(
        local_conf.global.internal_metrics_interval,
        INTERNAL_METRICS.processor_channel.clone(),
    );
    let (sink_admin, sink_send) = new_channel(
        local_conf.global.internal_metrics_interval,
        INTERNAL_METRICS.sink_channel.clone(),
    );
    let mut processors = processor::WorkerGroup::new(
        sink_send,
        processor_admin
            .get_receiver(
                &processor::PROCESSOR_CHANNEL_NAME,
                local_conf.global.processor.channel,
            )
            .await?,
        config_cache.clone(),
    );
    let listeners =
        listener::WorkerGroup::new(processor_send.clone(), None, config_cache.clone(), None);
    let monitors = monitor::WorkerGroup::new(
        processor_send,
        local_conf.monitor.batch,
        config_cache.clone(),
    );
    let sinks = sink::WorkerGroup::new(sink_admin);
    let sink_reporter = sinks
        .clone()
        .spawn_metrics_reporter(local_conf.global.internal_metrics_interval)
        .await;
    let monitor_reporter = monitors
        .clone()
        .spawn_metrics_reporter(local_conf.global.internal_metrics_interval)
        .await;
    let listener_reporter = listeners
        .clone()
        .spawn_metrics_reporter(local_conf.global.internal_metrics_interval)
        .await;
    let sink_supervisor = sinks.clone().spawn_supervisor();
    let listener_supervisor = listeners.clone().spawn_supervisor();
    processors
        .spawn_metrics_reporter(local_conf.global.internal_metrics_interval)
        .await;
    for sink in &local_conf.sinks {
        sinks.add(sink.clone()).await?;
    }
    // Sinks taken out of the file since the last run left their disk data.
    sinks.remove_orphaned_data().await;
    processors.run().await;
    for listener in &local_conf.listeners {
        listeners.add(listener.clone()).await?;
    }
    let monitor_run = monitors.clone().run().await;
    monitors.adjust(local_conf.monitor.worker_count).await;
    info!(
        listeners = local_conf.listeners.len(),
        sinks = local_conf.sinks.len(),
        items = local_conf.items.len(),
        "startup complete"
    );
    token.cancelled().await;
    let deadline = shutdown.deadline();
    info!(
        timeout_ms = local_conf.global.shutdown_timeout.as_millis() as u64,
        "shutting down"
    );
    join_step("lookup_reload", lookup_reloader).await;
    join_step("lookup_reporter", lookup_reporter).await;
    monitors.close_scheduler();
    join_step("monitor_run", monitor_run).await;
    monitors.close_metrics_reporter();
    join_step("monitor_reporter", monitor_reporter).await;
    drop(monitors);
    listeners.close(deadline).await;
    join_step("listener_reporter", listener_reporter).await;
    join_step("listener_supervisor", listener_supervisor).await;
    drop(listeners);
    drop(processor_admin);
    processors.close(deadline).await;
    sinks.close(deadline).await;
    join_step("sink_reporter", sink_reporter).await;
    join_step("sink_supervisor", sink_supervisor).await;
    drop(sinks);
    if let Some(prometheus_server) = prometheus_server {
        prometheus_server.wait().await?;
    }
    info!("shutdown complete");
    Ok(())
}

async fn backend_run(global_conf: config::global::Global) -> Result<()> {
    let backend_conf = global_conf.backend.clone().unwrap();
    let api_conf = backend_conf.api.clone();
    // Before the database: a missing secret must not get as far as migrations or the
    // cluster lock.
    let jwt = api_conf
        .enabled
        .then(|| {
            api::jwt::Jwt::from_env(
                api_conf.auth.local.jwt_issuer.clone(),
                api_conf.auth.local.access_token_ttl,
                api_conf.auth.local.refresh_token_ttl,
                api_conf.auth.oidc.as_ref(),
            )
        })
        .transpose()?;
    let secret_keys = Arc::new(config::sensitive::SecretKeys::from_env()?);
    let db_pool = match &backend_conf.db_tls {
        None => backend_conf
            .get_db_config()?
            .create_pool(Some(Runtime::Tokio1), tokio_postgres::NoTls)
            .context("failed to create database pool")?,
        Some(tls) => {
            let tls_config = common::tls::build_rustls_client_config(tls)
                .context("failed to build TLS config")?;
            let tls = MakeRustlsConnect::new(tls_config);
            backend_conf
                .get_db_config()?
                .create_pool(Some(Runtime::Tokio1), tls)
                .context("failed to create database pool with TLS")?
        }
    };
    // TODO: db_pool.status() -> incorporate to internal_metrics
    db::run_migrations(&db_pool).await?;
    db::reencrypt_secrets(&db_pool, &secret_keys).await?;
    let shutdown = setup_shutdown(global_conf.shutdown_timeout);
    let token = shutdown.token();
    let prometheus_server = global_conf
        .prometheus
        .as_ref()
        .map(|prometheus| {
            common::prometheus::spawn(
                prometheus,
                Arc::clone(&*INTERNAL_METRICS),
                true,
                token.clone(),
            )
        })
        .transpose()?;

    info!("initializing cluster");
    let cluster = cluster::Cluster::new(
        backend_conf.cluster.clone(),
        db_pool.clone(),
        &global_conf,
        shutdown.clone(),
        secret_keys.clone(),
    )
    .await?;

    let api_handle = if let Some(jwt) = jwt {
        let app_state = api::state::AppState::new(
            db_pool.clone(),
            cache::get_cache(),
            cluster.inter_comm(),
            Arc::new(jwt),
            secret_keys,
        );
        let app = api::create_router(app_state, &api_conf.cors);
        let handle = axum_server::Handle::new();
        let shutdown_handle = handle.clone();
        let backend_shutdown = shutdown.clone();
        tokio::spawn(async move {
            backend_shutdown.token().cancelled().await;
            info!("shutting down backend");
            // The cluster teardown waits for this, so it comes out of the
            // same deadline.
            let grace = backend_shutdown.deadline().budget(API_DRAIN_TIMEOUT);
            shutdown_handle.graceful_shutdown(Some(grace));
        });
        let api_token = token.clone();
        Some(match &api_conf.tls {
            None => {
                // the bind happens inside serve, so this can't claim to be listening yet
                info!(addr = %api_conf.addr, tls = false, "starting API server");
                tokio::spawn(async move {
                    let res = axum_server::bind(api_conf.addr)
                        .handle(handle)
                        .serve(app.into_make_service())
                        .await;
                    api_server_ended(res, api_conf.addr, &api_token)
                })
            }
            Some(tls) => {
                let server_tls_config = common::tls::build_rustls_server_config(tls)
                    .context("failed to build API server TLS config")?;
                let rustls_config = RustlsConfig::from_config(Arc::new(server_tls_config));

                // the bind happens inside serve, so this can't claim to be listening yet
                info!(addr = %api_conf.addr, tls = true, "starting API server");
                tokio::spawn(async move {
                    let res = axum_server::bind_rustls(api_conf.addr, rustls_config)
                        .handle(handle)
                        .serve(app.into_make_service())
                        .await;
                    api_server_ended(res, api_conf.addr, &api_token)
                })
            }
        })
    } else {
        info!("REST API disabled");
        None
    };
    let token_cleanup = api_handle.is_some().then(|| {
        info!("starting refresh token cleanup loop");
        api::jwt::spawn_expired_token_cleanup(db_pool.clone(), Duration::from_secs(60 * 60))
    });

    info!("starting cluster");
    let (cluster_handle, cluster_token) = cluster.run();
    info!("cluster running");
    // An API server that fails ends the node, but only after the teardown:
    // its error is returned last.
    let api_res = match api_handle {
        Some(handle) => {
            let res = join_step("api_server", handle).await.unwrap_or(Ok(()));
            info!("API server stopped");
            res
        }
        None => {
            token.cancelled().await;
            Ok(())
        }
    };
    if let Some((token_cleanup_handle, token_cleanup_token)) = token_cleanup {
        info!("shutting down refresh token cleanup task");
        token_cleanup_token.cancel();
        join_step("token_cleanup", token_cleanup_handle).await;
    }
    info!("shutting down cluster");
    cluster_token.cancel();
    join_step("cluster", cluster_handle).await;
    db_pool.close();
    if let Some(prometheus_server) = prometheus_server {
        prometheus_server.wait().await?;
    }
    api_res?;
    info!("shutdown complete");
    Ok(())
}

/// The API server's task result. Serving returns only on failure (a bind, a
/// TLS setup) or once the graceful shutdown is done; a failure starts the
/// shutdown, the way the Prometheus server's does.
fn api_server_ended(
    res: std::io::Result<()>,
    addr: std::net::SocketAddr,
    token: &CancellationToken,
) -> Result<()> {
    let Err(e) = res else {
        return Ok(());
    };
    error!(error = %e, addr = %addr, "API server failed");
    token.cancel();
    Err(anyhow::Error::new(e).context("API server failed"))
}

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls crypto provider"))?;
    // initialize uptime value
    let _ = common::uptime();
    let _ = common::get_start_datetime();
    let args = Args::parse();
    let content = fs::read_to_string(&args.config).await?;
    // Partial config only contains Global, the rest of the config needs MIB to be initialized
    let mut local_conf: config::PartialConfig = serde_yaml_ng::from_str(&content)?;
    if local_conf.global.logger.print_trace {
        unsafe {
            std::env::set_var("RUST_BACKTRACE", "1");
        }
    } else {
        unsafe {
            std::env::remove_var("RUST_BACKTRACE");
        }
    }
    // held here rather than by the run functions, so the fatal-error event
    // below is still flushed to the configured output, and so is a forced
    // exit's
    *LOG_GUARD.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(init_logging(&local_conf.global.logger));
    common::panic::install();
    // DEFAULT_TZ was already forced above, before logging existed, so its
    // fallback can only be reported now
    if let Ok(tz) = std::env::var("TZ")
        && tz.parse::<chrono_tz::Tz>().is_err()
    {
        warn!(tz = %tz, "invalid TZ, falling back to UTC");
    }
    let res = async move {
        if local_conf.global.backend.is_some() {
            // everything but `global` comes from the database in backend mode,
            // so anything else in the file would be silently ignored
            serde_yaml_ng::from_str::<config::BackendFileConfig>(&content)
                .context("backend mode reads only `global` from the config file")?;
        }
        // oid validate uses mib, so it's important to init mib first
        init_mib(&local_conf.global.mib).await;
        local_conf.validate()?;

        install_mem_profile_signals();

        chan::init_database(&local_conf.global.disk_channel)?;
        if let Some(backend) = &local_conf.global.backend {
            _ = config::NODE_ID.set(backend.cluster.node_id);
            backend_run(local_conf.global).await
        } else {
            local_run(content).await
        }
    }
    .await;
    if let Err(e) = &res {
        error!(error = %format!("{e:#}"), "fatal error");
    }
    if common::panic::panicked() {
        force_exit(common::panic::EXIT_CODE);
    }
    flush_logs();
    res
}

#[cfg(test)]
mod logging_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing::{error, subscriber};

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn lines(&self) -> usize {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|b| **b == b'\n')
                .count()
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    impl io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn throttle_keeps_the_targets_level_hint() {
        use tracing_subscriber::Registry;
        fn hint<F: Filter<Registry>>(f: &F) -> Option<LevelFilter> {
            f.max_level_hint()
        }
        let targets = || Targets::default().with_target("patrolli", LevelFilter::INFO);
        // Unwrapped, the throttle erases the hint and `log` falls back to TRACE.
        assert_eq!(hint(&targets().and(Some(build_throttle(false)))), None);
        assert_eq!(
            hint(&targets().and(NoLevelBound(Some(build_throttle(false))))),
            Some(LevelFilter::INFO)
        );
        // `disable_throttle`: a `None` filter has no hint either.
        assert_eq!(
            hint(&targets().and(NoLevelBound(None::<TracingRateLimitLayer>))),
            Some(LevelFilter::INFO)
        );
    }

    /// Builds the same filter stack `init_logging` installs, over a capturing writer.
    fn capturing_subscriber() -> (impl subscriber::Subscriber, Capture) {
        let capture = Capture::default();
        let targets = Targets::default().with_target("patrolli", LevelFilter::TRACE);
        let throttle = TracingRateLimitLayer::builder()
            .with_policy(Policy::token_bucket(50.0, 1.0).unwrap())
            .with_excluded_fields(vec![
                "error".to_string(),
                "request_id".to_string(),
                "stats".to_string(),
                "latency_ms".to_string(),
                "user".to_string(),
                "peer".to_string(),
                "batch_id".to_string(),
                "records".to_string(),
                "rows".to_string(),
                "reason".to_string(),
                "row_id".to_string(),
            ])
            .with_max_signatures(MAX_LOG_SIGNATURES)
            .with_summary_interval(Duration::from_secs(60))
            .with_active_emission(true)
            .build()
            .unwrap();
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(capture.clone())
            .with_ansi(false)
            .with_filter(targets.and(throttle));
        (tracing_subscriber::registry().with(layer), capture)
    }

    const BURST: usize = 500;

    /// The convention's whole mechanical justification: the throttle signature includes the
    /// rendered message text, so an error interpolated into the message makes every occurrence
    /// unique and nothing is ever suppressed. In a field named `error` — which `init_logging`
    /// excludes from the signature — the same failure collapses to one signature.
    // `with_active_emission(true)` spawns the summary emitter, so this needs a reactor --
    // the same reason `init_logging` is only ever called from inside `#[tokio::main]`.
    #[tokio::test]
    async fn an_error_in_a_field_is_throttled_but_one_in_the_message_is_not() {
        let (sub, capture) = capturing_subscriber();
        let in_field = subscriber::with_default(sub, || {
            for i in 0..BURST {
                error!(error = %format!("connection {i} refused"), "kafka send failed");
            }
            capture.lines()
        });

        let (sub, capture) = capturing_subscriber();
        let in_message = subscriber::with_default(sub, || {
            for i in 0..BURST {
                // log-style: allow — demonstrating the anti-pattern is the point of this test
                error!("kafka send failed: connection {i} refused");
            }
            capture.lines()
        });

        assert!(
            in_field < BURST / 2,
            "an error in a field should collapse to one signature and be throttled, got {in_field} of {BURST} lines"
        );
        assert_eq!(
            in_message, BURST,
            "an interpolated message defeats the throttle entirely -- this is why the convention \
             requires a constant message literal"
        );
    }
}
