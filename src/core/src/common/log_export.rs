// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Capture side of internal log export: a `tracing` layer that fans patrolli's own
//! events out to any number of registered consumers.
//!
//! `tracing` dispatches synchronously on the thread that logs, so everything in
//! [`LogExportLayer::on_event`] runs inside the caller's `warn!`. It therefore
//! never blocks, awaits, does I/O or logs: it matches the event against each
//! [`Registration`]'s filter, copies it once into an owned [`LogEvent`] and
//! `try_send`s it into that registration's bounded flume channel. A full
//! channel drops the event and counts it in [`Registration::dropped`]; turning
//! events into records is the consumer's job, on its own task.
//!
//! Span fields are recorded into an extension so an event carries the identity
//! of its scope (`sink_id`, `listener`, ...), which the logging convention puts
//! on spans rather than events.

use crate::config::global::{Level as ConfigLevel, LogModule};
use arc_swap::ArcSwap;
use smallvec::SmallVec;
use smol_str::{SmolStr, SmolStrBuilder};
use std::fmt::{self, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_log::{AsLog, NormalizeEvent};
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::{Context, Filter, Layer};
use tracing_subscriber::registry::LookupSpan;

/// The process-wide export point `init_logging` installs.
pub static LOG_EXPORT: LazyLock<Arc<LogExport>> = LazyLock::new(LogExport::new);

/// Spans at or above this level are always recorded, with or without a
/// registration. Long-lived task spans (listeners, sinks, monitors) are created
/// before an export consumer registers; skipping them would lose their identity
/// for the rest of the process. Less severe spans are recorded only while a
/// registration wants that level, because some sit on hot paths (the
/// processor's per-step span is DEBUG).
const ALWAYS_RECORDED_SPAN_LEVEL: LevelFilter = LevelFilter::INFO;

pub type Fields = SmallVec<[(&'static str, FieldValue); 8]>;

#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(SmolStr),
}

/// One captured event, owned so it can outlive the `tracing` call.
///
/// Level, target, file and line are copied out rather than kept as the event's
/// `&'static Metadata`: a record bridged from the `log` crate arrives under
/// tracing-log's per-level placeholder (target `log`, no file or line), and
/// its real values exist only for the duration of the call.
#[derive(Debug)]
pub struct LogEvent {
    pub timestamp: SystemTime,
    pub level: Level,
    /// The module the event came from: the `tracing` target, or the `log`
    /// record's own target.
    pub target: SmolStr,
    pub file: Option<SmolStr>,
    pub line: Option<u32>,
    pub message: SmolStr,
    /// The event's own fields, without `message`.
    pub fields: Fields,
    /// Fields of the enclosing spans, innermost first wins on a name clash.
    /// Kept apart from `fields` so the consumer decides precedence.
    pub span_fields: Fields,
}

struct Subscription {
    id: u64,
    filter: Targets,
    tx: flume::Sender<Arc<LogEvent>>,
    dropped: Arc<AtomicU64>,
}

struct State {
    subs: Vec<Arc<Subscription>>,
    /// Most verbose level any subscription enables, `OFF` when there are none.
    max_level: LevelFilter,
}

impl State {
    fn with_subs(subs: Vec<Arc<Subscription>>) -> Self {
        let max_level = subs
            .iter()
            .map(|s| targets_max_level(&s.filter))
            .max()
            .unwrap_or(LevelFilter::OFF);
        Self { subs, max_level }
    }
}

fn targets_max_level(targets: &Targets) -> LevelFilter {
    targets
        .iter()
        .map(|(_, level)| level)
        .chain(targets.default_level())
        .max()
        .unwrap_or(LevelFilter::OFF)
}

pub struct LogExport {
    state: ArcSwap<State>,
    next_id: AtomicU64,
}

impl LogExport {
    /// An isolated export point; tests use their own so parallel tests don't
    /// see each other's registrations.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: ArcSwap::from_pointee(State::with_subs(Vec::new())),
            next_id: AtomicU64::new(0),
        })
    }

    /// Starts delivering events that `filter` enables into a channel holding at
    /// most `buffer_size` events. Delivery stops when the returned
    /// [`Registration`] is dropped.
    pub fn register(
        self: &Arc<Self>,
        filter: Targets,
        buffer_size: usize,
    ) -> (Registration, flume::Receiver<Arc<LogEvent>>) {
        let (tx, rx) = flume::bounded(buffer_size.max(1));
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let dropped = Arc::new(AtomicU64::new(0));
        let sub = Arc::new(Subscription {
            id,
            filter,
            tx,
            dropped: dropped.clone(),
        });
        self.state.rcu(|state| {
            let mut subs = state.subs.clone();
            subs.push(sub.clone());
            State::with_subs(subs)
        });
        // Callsites cached as `never` (a `debug!` while every layer was at
        // info) stay dead until their interest is recomputed.
        rebuild_interest();
        let registration = Registration {
            id,
            export: self.clone(),
            dropped,
        };
        (registration, rx)
    }

    fn unregister(&self, id: u64) {
        self.state.rcu(|state| {
            State::with_subs(state.subs.iter().filter(|s| s.id != id).cloned().collect())
        });
        rebuild_interest();
    }

    #[cfg(test)]
    pub(crate) fn subscriptions(&self) -> usize {
        self.state.load().subs.len()
    }

    fn wants_event(&self, meta: &Metadata<'_>) -> bool {
        self.state
            .load()
            .subs
            .iter()
            .any(|s| s.filter.would_enable(meta.target(), meta.level()))
    }

    fn wants_span(&self, meta: &Metadata<'_>) -> bool {
        *meta.level() <= ALWAYS_RECORDED_SPAN_LEVEL || *meta.level() <= self.state.load().max_level
    }
}

/// Keeps a registration alive; dropping it stops delivery.
pub struct Registration {
    id: u64,
    export: Arc<LogExport>,
    dropped: Arc<AtomicU64>,
}

impl Registration {
    /// Events discarded because the channel was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.export.unregister(self.id);
    }
}

/// Builds a registration filter. An empty `include` exports `patrolli` at
/// `level`; otherwise each module at its own level. Each `exclude` is turned
/// off, and since `Targets` resolves by longest prefix an exclusion can carve
/// a submodule out of an include.
pub fn targets(level: ConfigLevel, include: &[LogModule], exclude: &[String]) -> Targets {
    let mut targets = if include.is_empty() {
        Targets::new().with_target("patrolli", level)
    } else {
        Targets::new().with_targets(include.iter().map(|m| (m.name.clone(), m.level)))
    };
    for module in exclude {
        targets = targets.with_target(module.clone(), LevelFilter::OFF);
    }
    targets
}

pub fn layer(export: Arc<LogExport>) -> LogExportLayer {
    LogExportLayer { export }
}

pub fn filter(export: Arc<LogExport>) -> LogExportFilter {
    LogExportFilter { export }
}

/// The per-layer filter for [`LogExportLayer`]. Spans must pass it too:
/// per-layer filtering hides a span this filter rejects from the layer's
/// `event_scope`, taking its fields with it.
pub struct LogExportFilter {
    export: Arc<LogExport>,
}

/// Recomputes callsite interest and the global max level after a registration
/// change, and carries that level over to `log`.
///
/// `log` records have no callsite to cache `never` on, so `log`'s own max level
/// is the only early cut-off they get. `init_logging` sets it from the
/// subscriber at startup; without following registrations, a consumer asking
/// for a `log`-based crate below that level would get nothing. The lock spans
/// both steps so concurrent changes cannot leave the level computed by the
/// older one.
fn rebuild_interest() {
    static LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let _guard = LOCK.lock();
    tracing::callsite::rebuild_interest_cache();
    tracing_log::log::set_max_level(LevelFilter::current().as_log());
}

impl LogExportFilter {
    fn is_enabled(&self, meta: &Metadata<'_>) -> bool {
        if meta.is_span() {
            self.export.wants_span(meta)
        } else {
            self.export.wants_event(meta)
        }
    }
}

impl<S> Filter<S> for LogExportFilter {
    fn enabled(&self, meta: &Metadata<'_>, _cx: &Context<'_, S>) -> bool {
        self.is_enabled(meta)
    }

    // `always`/`never` is safe to cache: every registration change calls
    // `rebuild_interest_cache`.
    fn callsite_enabled(&self, meta: &'static Metadata<'static>) -> Interest {
        if self.is_enabled(meta) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(ALWAYS_RECORDED_SPAN_LEVEL.max(self.export.state.load().max_level))
    }
}

/// Span fields, kept as a span extension.
struct SpanFields(Fields);

pub struct LogExportLayer {
    export: Arc<LogExport>,
}

impl<S> Layer<S> for LogExportLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut fields = Fields::new();
        attrs.record(&mut Collector {
            message: None,
            fields: &mut fields,
            skip_log_fields: false,
        });
        span.extensions_mut().replace(SpanFields(fields));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut ext = span.extensions_mut();
        if let Some(SpanFields(fields)) = ext.get_mut::<SpanFields>() {
            values.record(&mut Collector {
                message: None,
                fields,
                skip_log_fields: false,
            });
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let state = self.export.state.load();
        // A `log` record carries its real target, file and line in `log.*`
        // fields of a placeholder callsite; match and record on those.
        let normalized = event.normalized_metadata();
        let meta = normalized.as_ref().unwrap_or_else(|| event.metadata());
        let matched: SmallVec<[&Arc<Subscription>; 4]> = state
            .subs
            .iter()
            .filter(|s| s.filter.would_enable(meta.target(), meta.level()))
            .collect();
        if matched.is_empty() {
            return;
        }

        let mut message = SmolStr::default();
        let mut fields = Fields::new();
        event.record(&mut Collector {
            message: Some(&mut message),
            fields: &mut fields,
            skip_log_fields: normalized.is_some(),
        });
        let mut span_fields = Fields::new();
        if let Some(scope) = ctx.event_scope(event) {
            // leaf to root: the innermost span's value wins
            for span in scope {
                let ext = span.extensions();
                let Some(SpanFields(own)) = ext.get::<SpanFields>() else {
                    continue;
                };
                for (name, value) in own {
                    if !span_fields.iter().any(|(n, _)| n == name) {
                        span_fields.push((name, value.clone()));
                    }
                }
            }
        }
        let (target, file) = match &normalized {
            Some(m) => (SmolStr::new(m.target()), m.file().map(SmolStr::new)),
            None => {
                let m = event.metadata();
                (
                    SmolStr::new_static(m.target()),
                    m.file().map(SmolStr::new_static),
                )
            }
        };
        let ev = Arc::new(LogEvent {
            timestamp: SystemTime::now(),
            level: *meta.level(),
            target,
            file,
            line: meta.line(),
            message,
            fields,
            span_fields,
        });

        for sub in matched {
            if let Err(flume::TrySendError::Full(_)) = sub.tx.try_send(ev.clone()) {
                sub.dropped.fetch_add(1, Ordering::Relaxed);
            }
            // `Disconnected`: the consumer is going away and its registration
            // is about to be dropped.
        }
    }
}

/// Records fields into `fields`, replacing a same-named entry. `message` is
/// routed to its own slot when one is given. `skip_log_fields` drops the
/// `log.*` fields of a bridged `log` record, which [`LogEvent`] already holds
/// as its target, file and line.
struct Collector<'a> {
    message: Option<&'a mut SmolStr>,
    fields: &'a mut Fields,
    skip_log_fields: bool,
}

impl Collector<'_> {
    fn put(&mut self, field: &Field, value: FieldValue) {
        let name = field.name();
        if self.skip_log_fields && name.starts_with("log.") {
            return;
        }
        if name == "message"
            && let Some(message) = self.message.as_deref_mut()
        {
            *message = match value {
                FieldValue::Str(s) => s,
                other => render(format_args!("{other:?}")),
            };
            return;
        }
        match self.fields.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => self.fields.push((name, value)),
        }
    }
}

fn render(args: fmt::Arguments<'_>) -> SmolStr {
    let mut builder = SmolStrBuilder::new();
    let _ = builder.write_fmt(args);
    builder.finish()
}

impl Visit for Collector<'_> {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, FieldValue::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, FieldValue::I64(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, FieldValue::U64(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, FieldValue::F64(value));
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        self.put(field, FieldValue::Str(render(format_args!("{value}"))));
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        self.put(field, FieldValue::Str(render(format_args!("{value}"))));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, FieldValue::Str(SmolStr::new(value)));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.put(field, FieldValue::Str(render(format_args!("{value:?}"))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::{debug, info, info_span, warn};
    use tracing_subscriber::prelude::*;

    fn with_export<R>(export: &Arc<LogExport>, f: impl FnOnce() -> R) -> R {
        let subscriber = tracing_subscriber::registry()
            .with(layer(export.clone()).with_filter(filter(export.clone())));
        tracing::subscriber::with_default(subscriber, f)
    }

    fn drain(rx: &flume::Receiver<Arc<LogEvent>>) -> Vec<Arc<LogEvent>> {
        rx.try_iter().collect()
    }

    fn field<'a>(fields: &'a Fields, name: &str) -> Option<&'a FieldValue> {
        fields.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    fn patrolli_info() -> Targets {
        targets(ConfigLevel::Info, &[], &[])
    }

    /// Routes `log` records into `tracing`, as `init_logging` does. Process-wide
    /// and settable once, so every test that needs it shares one install.
    fn install_log_tracer() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| tracing_log::LogTracer::init().expect("log tracer"));
    }

    fn include(name: &str, level: ConfigLevel) -> Targets {
        targets(
            ConfigLevel::Info,
            &[LogModule {
                name: name.into(),
                level,
            }],
            &[],
        )
    }

    #[test]
    fn log_records_export_under_their_own_target() {
        install_log_tracer();
        let export = LogExport::new();
        // Registered with the subscriber alive, as in production: `log`'s max
        // level is derived from live subscribers, and other tests' changes
        // re-derive it at any moment.
        let (_reg, rx, line) = with_export(&export, || {
            let (reg, rx) = export.register(include("fake_dep", ConfigLevel::Warn), 16);
            tracing_log::log::warn!(target: "other_dep", "not included");
            tracing_log::log::warn!(target: "fake_dep::storage", "disk {} slow", 3);
            (reg, rx, line!() - 1)
        });
        let events = drain(&rx);
        assert_eq!(events.len(), 1, "{events:?}");
        let ev = &events[0];
        assert_eq!(ev.target, "fake_dep::storage");
        assert_eq!(ev.level, tracing::Level::WARN);
        assert_eq!(ev.message, "disk 3 slow");
        assert_eq!(ev.file.as_deref(), Some(file!()));
        assert_eq!(ev.line, Some(line));
        // tracing-log's placeholder fields are already target, file and line.
        assert!(ev.fields.is_empty(), "{:?}", ev.fields);
    }

    #[test]
    fn a_registration_raises_the_log_max_level() {
        install_log_tracer();
        let export = LogExport::new();
        with_export(&export, || {
            // Where `init_logging` leaves it with nothing asking for more.
            tracing_log::log::set_max_level(tracing_log::log::LevelFilter::Info);
            let (_reg, rx) = export.register(include("fake_dep", ConfigLevel::Debug), 16);
            assert!(tracing_log::log::max_level() >= tracing_log::log::LevelFilter::Debug);
            tracing_log::log::debug!(target: "fake_dep::cache", "evicted");
            let messages: Vec<_> = drain(&rx).iter().map(|e| e.message.clone()).collect();
            assert_eq!(messages, ["evicted"]);
        });
    }

    #[test]
    fn nothing_is_captured_without_a_registration() {
        let export = LogExport::new();
        with_export(&export, || {
            info!(target: "patrolli::a", "nobody listens");
        });
        assert!(export.state.load().subs.is_empty());
        assert_eq!(export.state.load().max_level, LevelFilter::OFF);
    }

    #[test]
    fn include_and_exclude_select_modules() {
        let export = LogExport::new();
        let include = [
            LogModule {
                name: "patrolli".into(),
                level: ConfigLevel::Info,
            },
            LogModule {
                name: "patrolli::cluster".into(),
                level: ConfigLevel::Debug,
            },
        ];
        let (_reg, rx) = export.register(
            targets(
                ConfigLevel::Info,
                &include,
                &["patrolli::sink::kafka".into()],
            ),
            16,
        );
        with_export(&export, || {
            info!(target: "patrolli::sink::kafka::producer", "excluded");
            info!(target: "patrolli::sink::file", "included");
            info!(target: "other_crate", "not included");
            debug!(target: "patrolli::sink::file", "below level");
            debug!(target: "patrolli::cluster::node_lock", "debug include");
        });
        let messages: Vec<_> = drain(&rx).iter().map(|e| e.message.clone()).collect();
        assert_eq!(messages, ["included", "debug include"]);
    }

    #[test]
    fn empty_include_defaults_to_patrolli() {
        let export = LogExport::new();
        let (_reg, rx) = export.register(patrolli_info(), 16);
        with_export(&export, || {
            info!(target: "patrolli::x", "ours");
            info!(target: "hyper::proto", "theirs");
        });
        let events = drain(&rx);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].target, "patrolli::x");
    }

    #[test]
    fn a_full_buffer_drops_and_counts() {
        let export = LogExport::new();
        let (reg, rx) = export.register(patrolli_info(), 2);
        with_export(&export, || {
            for i in 0..5 {
                info!(target: "patrolli::x", i, "burst");
            }
        });
        assert_eq!(drain(&rx).len(), 2);
        assert_eq!(reg.dropped(), 3);
    }

    #[test]
    fn fields_keep_their_types() {
        let export = LogExport::new();
        let (_reg, rx) = export.register(patrolli_info(), 16);
        let err = std::io::Error::other("boom");
        with_export(&export, || {
            warn!(
                target: "patrolli::x",
                ok = true,
                signed = -3i64,
                unsigned = 7u64,
                ratio = 0.5f64,
                name = "kafka",
                error = %err,
                "send failed"
            );
        });
        let events = drain(&rx);
        let ev = &events[0];
        assert_eq!(ev.message, "send failed");
        assert_eq!(ev.level, tracing::Level::WARN);
        assert_eq!(field(&ev.fields, "ok"), Some(&FieldValue::Bool(true)));
        assert_eq!(field(&ev.fields, "signed"), Some(&FieldValue::I64(-3)));
        assert_eq!(field(&ev.fields, "unsigned"), Some(&FieldValue::U64(7)));
        assert_eq!(field(&ev.fields, "ratio"), Some(&FieldValue::F64(0.5)));
        assert_eq!(
            field(&ev.fields, "name"),
            Some(&FieldValue::Str("kafka".into()))
        );
        assert_eq!(
            field(&ev.fields, "error"),
            Some(&FieldValue::Str("boom".into()))
        );
        assert!(field(&ev.fields, "message").is_none());
    }

    #[test]
    fn span_fields_innermost_wins_and_follow_record() {
        let export = LogExport::new();
        let (_reg, rx) = export.register(patrolli_info(), 16);
        with_export(&export, || {
            let outer = info_span!(target: "patrolli::sink", "sink", sink_id = 1u64, sink = "outer", host = tracing::field::Empty);
            let _o = outer.enter();
            outer.record("host", "h1");
            let inner = info_span!(target: "patrolli::sink", "batch", sink = "inner");
            let _i = inner.enter();
            info!(target: "patrolli::sink", "delivered");
        });
        let events = drain(&rx);
        let spans = &events[0].span_fields;
        assert_eq!(field(spans, "sink_id"), Some(&FieldValue::U64(1)));
        assert_eq!(field(spans, "sink"), Some(&FieldValue::Str("inner".into())));
        assert_eq!(field(spans, "host"), Some(&FieldValue::Str("h1".into())));
    }

    #[test]
    fn a_span_created_before_registering_still_contributes() {
        let export = LogExport::new();
        with_export(&export, || {
            let span = info_span!(target: "patrolli::listener", "listen", listener = "syslog");
            let _e = span.enter();
            let (_reg, rx) = export.register(patrolli_info(), 16);
            info!(target: "patrolli::listener", "packet dropped");
            let events = drain(&rx);
            assert_eq!(
                field(&events[0].span_fields, "listener"),
                Some(&FieldValue::Str("syslog".into()))
            );
        });
    }

    #[test]
    fn registrations_get_their_own_subset_of_one_event() {
        let export = LogExport::new();
        let (_all, rx_all) = export.register(patrolli_info(), 16);
        let (_warn, rx_warn) = export.register(targets(ConfigLevel::Warn, &[], &[]), 16);
        with_export(&export, || {
            info!(target: "patrolli::x", "info");
            warn!(target: "patrolli::x", "warn");
        });
        let all = drain(&rx_all);
        let warn = drain(&rx_warn);
        assert_eq!(all.len(), 2);
        assert_eq!(warn.len(), 1);
        assert!(Arc::ptr_eq(&all[1], &warn[0]));
    }

    #[test]
    fn dropping_the_registration_stops_delivery() {
        let export = LogExport::new();
        let (reg, rx) = export.register(patrolli_info(), 16);
        with_export(&export, || {
            info!(target: "patrolli::x", "before");
            drop(reg);
            info!(target: "patrolli::x", "after");
        });
        assert!(export.state.load().subs.is_empty());
        let messages: Vec<_> = drain(&rx).iter().map(|e| e.message.clone()).collect();
        assert_eq!(messages, ["before"]);
    }
}
