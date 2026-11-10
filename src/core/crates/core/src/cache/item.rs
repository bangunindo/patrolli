// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::common;
use crate::config::item;
use anyhow::Result;
use arc_swap::ArcSwapOption;
use bytes::Bytes;
use chrono::{DateTime, FixedOffset, TimeDelta};
use parking_lot::RwLock;
use priority_queue::PriorityQueue;
use rustc_hash::{FxBuildHasher, FxHashMap};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::cmp::Reverse;
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};
use std::sync::atomic::{AtomicI64, AtomicU8, AtomicU32, AtomicU64};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::time::Instant;
use tracing::{debug, info, trace};
use utoipa::ToSchema;
use uuid::Uuid;

/// Target of the item state-transition stream emitted by [`Transition::log`].
///
/// Deliberately outside `patrolli::cache::item`: it is a stream of its own, meant to
/// be filtered (`global.logger.modules`) and exported independently, so a filter on
/// the cache module must not catch it and a filter on it must not catch the cache.
///
/// Only transitions a writer makes are logged. [`ItemState::Stale`] is derived on
/// read and never written, so it never appears here — detecting it would mean
/// sweeping every item on a timer, which does not scale to a million items.
pub const STATE_LOG_TARGET: &str = "patrolli::item_state";

pub(super) static ITEM_CACHE: LazyLock<Arc<RwLock<Item>>> =
    LazyLock::new(|| Arc::new(RwLock::new(Item::new())));

/// Per-item health, written by whoever executes the item and read by the stats
/// API. Lock-free: there is no `RwLock` around this, so a reader never waits on
/// a check and a check never waits on a reader.
///
/// Counters use `fetch_add`/`fetch_or` rather than load-then-store. Monitor
/// items are single-writer by construction (`pop_due` takes the item out of the
/// queue and only `requeue` puts it back, and [`Item::execute_now`] refuses an
/// item that is in flight), so a plain store would be sound for them — but
/// [`Stats::add_dropped`] is called from the batcher task while the worker is
/// still running, and listener items are genuinely multi-writer. An uncontended
/// `lock xadd` costs nothing at this cadence, so the same API is correct in all
/// three cases.
pub struct Stats {
    total_checks: AtomicU64,
    total_failures: AtomicU64,
    total_records: AtomicU64,
    total_dropped: AtomicU64,
    consecutive_failures: AtomicU32,
    retries: AtomicU8,
    state: AtomicU8,

    // unix millis, 0 = never (can't put DateTime in an atomic)
    last_attempt_ms: AtomicI64,
    last_success_ms: AtomicI64,
    last_nonempty_ms: AtomicI64,
    next_check_ms: AtomicI64,
    /// Deadline past which a scheduled item reads as [`ItemState::Stale`]:
    /// `next_check_ms` plus a grace window. 0 = never stale.
    ///
    /// Staleness can't be written by the execution path — an item is stale
    /// precisely because nothing is executing it — so it is stamped by
    /// [`Item::requeue`], the one place that knows both the new `next_check`
    /// and the item's timeout, and evaluated on read by
    /// [`Stats::snapshot_at`].
    stale_after_ms: AtomicI64,

    last_duration_ms: AtomicU32,
    last_lag_ms: AtomicU32,
    last_record_count: AtomicU32,

    activity_window: ActivityWindow,

    // coupled cold fields: one publish, no torn pairs, allocates only on failure
    last_failure: ArcSwapOption<Failure>,
    recent_failures: ArcSwapOption<Vec<Failure>>, // copy-on-write ring, cap ~10
}

impl Default for Stats {
    /// A new item starts at [`ItemState::NeverRan`], not `Unknown`: the cache
    /// knows about it, the scheduler simply hasn't reached it yet. `Unknown` is
    /// reserved for a state byte we failed to interpret.
    fn default() -> Self {
        Self {
            total_checks: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
            total_records: AtomicU64::new(0),
            total_dropped: AtomicU64::new(0),
            consecutive_failures: AtomicU32::new(0),
            retries: AtomicU8::new(0),
            state: AtomicU8::new(ItemState::NeverRan as u8),

            last_attempt_ms: AtomicI64::new(0),
            last_success_ms: AtomicI64::new(0),
            last_nonempty_ms: AtomicI64::new(0),
            next_check_ms: AtomicI64::new(0),
            stale_after_ms: AtomicI64::new(0),

            last_duration_ms: AtomicU32::new(0),
            last_lag_ms: AtomicU32::new(0),
            last_record_count: AtomicU32::new(0),

            activity_window: ActivityWindow::default(),

            last_failure: ArcSwapOption::empty(),
            recent_failures: ArcSwapOption::empty(),
        }
    }
}

/// How many buckets the windowed activity map holds: 24 hours at 5-minute
/// resolution.
pub const ACTIVITY_BUCKETS: usize = 288;

/// Bytes one plane of [`ACTIVITY_BUCKETS`] occupies in a [`StatsSnapshot`].
pub const ACTIVITY_PLANE_BYTES: usize = ACTIVITY_BUCKETS.div_ceil(8);

/// [`ACTIVITY_BUCKETS`] bits rounded up to whole `u64`s. The top 32 bits of the
/// last word are unused and always zero. Internal: the atomics are words, but
/// nothing outside this module sees them — a snapshot carries packed bytes.
const ACTIVITY_WORDS: usize = ACTIVITY_BUCKETS.div_ceil(64);

/// Milliseconds per bucket.
const ACTIVITY_BUCKET_MS: i64 = 5 * 60 * 1000;

/// The trailing 24 hours of an item's activity, as a ring of 288 five-minute
/// buckets.
///
/// **Two planes, not one bit per bucket.** A single bit can't tell "failed" from
/// "wasn't due", and "wasn't due" is the common case: anything on a schedule
/// slower than five minutes leaves most buckets with no result at all, and even
/// the default `Interval(300s)` drifts, because `update_next_check` applies
/// per-item jitter. One bit per bucket would render a healthy hourly item as 264
/// failures out of 288.
///
/// So a bucket reads as four states, and every one of them is useful:
///
/// | `ok` | `fail` | meaning |
/// |---|---|---|
/// | 0 | 0 | no result — young item, or not due, or gone silent |
/// | 1 | 0 | healthy |
/// | 0 | 1 | failing |
/// | 1 | 1 | flapped inside the five minutes |
///
/// **Indexing.** A bucket is addressed by absolute 5-minute tick modulo 288, so
/// bit positions are stable: 288 × 5 min is exactly 24 h and the unix epoch is
/// UTC-midnight aligned, which makes bucket 0 always 00:00–00:05 **UTC**. No
/// timezone is involved anywhere on this path — no DST-length days, no
/// dependence on `$TZ`, and every node in a cluster agrees on what bit 17 means.
/// A viewer that wants local labels rotates by its own offset, which is lossless
/// because a full trailing day is always present.
///
/// Bucket `i` is bit `i % 64` of word `i / 64`: **ascending time, LSB first**.
///
/// **Why `last_tick`.** Because the ring is only 24 h around, a bucket's bits are
/// meaningless without knowing whether they were written this time around or a day
/// ago. `last_tick` is the newest bucket anyone has written, and it does two jobs:
/// a writer that advances past it clears the buckets it skipped (so stale bits
/// never merge with fresh ones), and a reader masks everything after it (so an
/// item that has gone silent shows an empty tail rather than a day-old strip that
/// looks current). Storing an *absolute* tick rather than a position-within-the-day
/// is load-bearing: a day position aliases at exactly 24 h, which would silently
/// merge consecutive runs of any `Cron` item scheduled once a day or slower.
///
/// Writes inside the newest bucket are a plain `fetch_or`, so they're idempotent
/// and commutative: a busy listener item that matches a thousand batches in one
/// bucket sets the same bit a thousand times and needs no retry loop. It also
/// means two nodes' planes merge with a plain `|`, should the same item ever
/// accumulate stats in more than one place.
#[derive(Default)]
struct ActivityWindow {
    ok: [AtomicU64; ACTIVITY_WORDS],
    fail: [AtomicU64; ACTIVITY_WORDS],
    /// Absolute 5-minute tick of the newest bucket written.
    ///
    /// Needs no "never written" sentinel: an all-zero plane pair already means
    /// "nothing recorded", so tick 0 is indistinguishable from no activity, which
    /// is the correct reading either way.
    last_tick: AtomicI64,
}

impl ActivityWindow {
    /// Both planes as of `now_tick`, packed for a snapshot, with the buckets after
    /// `last_tick` masked out.
    ///
    /// The writer only recycles a bucket when it advances onto it, so an item that
    /// has stopped reporting leaves the buckets between its last write and now
    /// holding a day-old strip. Reporting those verbatim would make a dead item
    /// look busy, so they are masked to "no result" — which is exactly what they
    /// are. Masking here rather than making the consumer intersect the planes
    /// against a "valid range" is the whole point: a snapshot is already true.
    fn read_masked(&self, now_tick: i64) -> (Bytes, Bytes) {
        let last = self.last_tick.load(Acquire);
        // Clamped, so a clock that went backwards masks nothing rather than
        // wrapping into a full sweep.
        let stale = (now_tick - last).clamp(0, ACTIVITY_BUCKETS as i64);
        if stale >= ACTIVITY_BUCKETS as i64 {
            return (Bytes::new(), Bytes::new());
        }
        let mut ok = read_plane(&self.ok);
        let mut fail = read_plane(&self.fail);
        for t in (now_tick - stale + 1)..=now_tick {
            clear_bucket(&mut ok, &mut fail, activity_bucket(t));
        }
        (pack_plane(&ok), pack_plane(&fail))
    }
}

/// The absolute 5-minute tick an instant falls in. `div_euclid` so pre-epoch
/// instants floor instead of truncating toward zero.
fn activity_tick(now_ms: i64) -> i64 {
    now_ms.div_euclid(ACTIVITY_BUCKET_MS)
}

/// Ring slot for an absolute tick.
fn activity_bucket(tick: i64) -> usize {
    tick.rem_euclid(ACTIVITY_BUCKETS as i64) as usize
}

/// Read a plane into plain words for a snapshot.
fn read_plane(plane: &[AtomicU64; ACTIVITY_WORDS]) -> [u64; ACTIVITY_WORDS] {
    std::array::from_fn(|i| plane[i].load(Relaxed))
}

/// Drop the bits of `bucket` from an in-memory plane pair.
fn clear_bucket(ok: &mut [u64; ACTIVITY_WORDS], fail: &mut [u64; ACTIVITY_WORDS], bucket: usize) {
    let mask = !(1u64 << (bucket % 64));
    ok[bucket / 64] &= mask;
    fail[bucket / 64] &= mask;
}

/// Pack a plane into [`ACTIVITY_PLANE_BYTES`] bytes, LSB-first: bucket `i` becomes
/// bit `i % 8` of byte `i / 8`.
///
/// Each word's little-endian bytes already lay its buckets out lowest-first, so
/// concatenating them and truncating away the unused tail of the last word gives
/// exactly that layout — no bit loop needed, and word 4's 32 padding bits are gone
/// by construction rather than by discipline.
///
/// An all-zero plane packs as empty rather than 36 zero bytes. Iterating an empty
/// plane yields no set buckets, which is the correct rendering, so a consumer needs
/// no special case — but `is_empty()` answers "did this item report anything in the
/// last 24 h?" in one call, and a stats read over many idle items allocates nothing.
fn pack_plane(words: &[u64; ACTIVITY_WORDS]) -> Bytes {
    if words.iter().all(|w| *w == 0) {
        return Bytes::new();
    }
    let mut out = Vec::with_capacity(ACTIVITY_WORDS * 8);
    for word in words {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.truncate(ACTIVITY_PLANE_BYTES);
    out.into()
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::IntoStaticStr,
)]
#[repr(u8)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Deadline passed with no answer. Item `timeout_ms` elapsed, SNMP
    /// `Timeout`, HTTP connect/read timeout.
    /// → raise the timeout, or the target is overloaded / silently filtered.
    Timeout = 1,
    /// Couldn't deliver the request at all: network error, socket closed,
    /// connection refused, DNS failure.
    /// → routing, ACLs, is the target even up.
    Unreachable = 2,
    /// Target rejected our credentials: SNMP `Auth`, HTTP 401/403.
    /// → the endpoint's community / v3 user / bearer token.
    Auth = 3,
    /// Target answered "not available to you": noSuchName, noAccess,
    /// authorizationError, OID outside the agent's view.
    /// → the item's OID, or the device's VACM view.
    NotSupported = 4,
    /// Target answered with an error of its own: genErr, tooBig, HTTP 5xx.
    /// → usually the device; sometimes fixed by smaller bulk repetitions.
    TargetError = 5,
    /// Target answered with bytes we can't use: malformed BER, walk returned
    /// non-increasing or cyclic OIDs, unparseable exposition format.
    /// → misbehaving agent/exporter; try disable_bulk.
    Malformed = 6,
    /// patrolli-side config is wrong or incomplete: endpoint missing or disabled,
    /// endpoint type doesn't match the item type, unparseable OID, bad TLS
    /// cert path. Never retryable.
    /// → the item or endpoint definition.
    Config = 7,
    /// Records were collected but couldn't be handed on — processor channel
    /// full, disk channel at its limit.
    /// → patrolli capacity, not the device.
    Dropped = 8,
    /// Forwarding to the node that owns this item failed.
    /// → the peer node / inter-cluster link.
    Cluster = 9,
    /// A patrolli bug: task panicked, impossible item/config pairing.
    /// → not the user's problem; this one's on you.
    Internal = 10,
}

/// Coarse health of an item, computed once on the write path so that every
/// reader (gRPC, REST, the Prometheus exporter) agrees without re-deriving it
/// from timestamps and cron schedules.
///
/// Discriminants match `v1::ItemState` one-for-one, so the conversion is a
/// cast. `Unknown` is 0 so a freshly zeroed `AtomicU8` is valid.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::IntoStaticStr,
)]
#[repr(u8)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    #[default]
    Unknown = 0,
    /// In the cache, but has never run a check or matched a record.
    NeverRan = 1,
    Ok = 2,
    /// Last check succeeded and produced zero records.
    OkEmpty = 3,
    /// Failing, with retries left before the check is abandoned.
    Retrying = 4,
    Failing = 5,
    /// Past due by more than its schedule allows — the scheduler or the node is
    /// wedged, which is not the same as the target failing.
    Stale = 6,
}

impl ItemState {
    /// Total order for the API's fan-out merge: when several nodes report the
    /// same listener item, the worst state wins.
    pub fn severity(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Ok => 1,
            Self::OkEmpty => 2,
            Self::NeverRan => 3,
            Self::Retrying => 4,
            Self::Stale => 5,
            Self::Failing => 6,
        }
    }
}

impl From<u8> for ItemState {
    /// Never panics: an out-of-range byte reads back as `Unknown`. This is on a
    /// diagnostics path, so a corrupt state must not take down a stats query.
    fn from(v: u8) -> Self {
        match v {
            1 => Self::NeverRan,
            2 => Self::Ok,
            3 => Self::OkEmpty,
            4 => Self::Retrying,
            5 => Self::Failing,
            6 => Self::Stale,
            _ => Self::Unknown,
        }
    }
}

/// `PartialEq` compares `at` by instant, not by rendered offset — that is what
/// `DateTime` equality means, and it is the right notion here: the wire form carries
/// an instant and no offset, so two failures that differ only in the zone they are
/// displayed in are the same failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Failure {
    pub at: DateTime<FixedOffset>,
    pub kind: FailureKind,
    #[schema(value_type = String)]
    pub reason: SmolStr, // truncate on write
    pub detail: Option<Box<serde_json::Value>>, // failing OIDs, http status
}

/// One read of a [`Stats`], flattened for transport.
///
/// Cheap to produce: the failure fields are `Arc` clones, not deep copies, so a
/// snapshot of an item with a full ring costs two atomic increments.
///
/// Everything here is in its final consumable form, because a snapshot is read far
/// more often than it is produced and every consumer would otherwise repeat the
/// same conversions. Timestamps are `DateTime<FixedOffset>` carrying the offset the
/// caller passed to [`Stats::snapshot_at`] — the same shape and offset as
/// [`Failure::at`], so the fields are directly comparable — and the activity planes
/// are packed bytes with the stale tail already masked off. The atomics behind all
/// of this are still millis and words; that's a storage detail, not an interface.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct StatsSnapshot {
    pub state: ItemState,

    pub total_checks: u64,
    pub total_failures: u64,
    pub total_records: u64,
    pub total_dropped: u64,
    pub consecutive_failures: u32,
    pub retries: u8,

    /// `None` where the event hasn't happened yet, so callers can tell "never
    /// succeeded" from "succeeded at the epoch".
    pub last_attempt: Option<DateTime<FixedOffset>>,
    pub last_success: Option<DateTime<FixedOffset>>,
    pub last_nonempty: Option<DateTime<FixedOffset>>,
    pub next_check: Option<DateTime<FixedOffset>>,

    /// `None` until the first check completes. A `Some(0)` record count is
    /// meaningful: the check ran and returned nothing.
    pub last_duration_ms: Option<u32>,
    pub last_lag_ms: Option<u32>,
    pub last_record_count: Option<u32>,

    /// The trailing 24 hours as 288 five-minute buckets, ending at
    /// `activity_window_end`. [`ACTIVITY_PLANE_BYTES`] each, LSB-first — bucket `i`
    /// is bit `i % 8` of byte `i / 8` — or empty when the item recorded nothing in
    /// the window.
    ///
    /// Buckets address absolute time by tick modulo 288, so bucket 0 is always
    /// 00:00–00:05 UTC and every node agrees on what a position means; see
    /// [`ActivityWindow`]. Buckets the item has not reached yet are already masked
    /// off, so what is here is what is true.
    ///
    /// A bucket set in neither plane had no result at all — a young item, a bucket
    /// it wasn't due in, or one it went silent through. That is **not** a failure,
    /// and for anything on a schedule slower than five minutes it is most buckets.
    #[schema(value_type = Vec<u8>)]
    pub activity_ok: Bytes,
    #[schema(value_type = Vec<u8>)]
    pub activity_fail: Bytes,
    /// Start of the newest bucket in the window: the 5-minute boundary at or
    /// before the read. The window covers the 288 buckets ending here, so bucket
    /// `i` is the most recent tick `t <= end` with `t % 288 == i`.
    pub activity_window_end: DateTime<FixedOffset>,

    #[schema(value_type = Option<Failure>)]
    pub last_failure: Option<Arc<Failure>>,
    #[schema(value_type = Option<Vec<Failure>>)]
    pub recent_failures: Option<Arc<Vec<Failure>>>,
}

/// How many failures the per-item ring keeps. Enough to see a pattern, small
/// enough that the copy-on-write clone on every failure stays trivial.
const MAX_RECENT_FAILURES: usize = 10;

/// Cap on [`Failure::reason`]. Long enough for a real error message, short
/// enough that a full ring on a million items stays bounded.
const MAX_REASON_LEN: usize = 200;

/// Milliseconds, clamped rather than wrapped — a pathological duration should
/// read as "very large", not as a small number.
fn as_millis_u32(d: Duration) -> u32 {
    d.as_millis().min(u32::MAX as u128) as u32
}

impl Failure {
    /// Build a failure, truncating `reason` to [`MAX_REASON_LEN`] on a character
    /// boundary.
    ///
    /// Truncating here rather than at each call site is deliberate: `reason` is
    /// shown verbatim in the UI and is fed by error strings from SNMP, reqwest
    /// and the TLS stack, none of which promise a bounded length.
    pub fn new(
        at: DateTime<FixedOffset>,
        kind: FailureKind,
        reason: &str,
        detail: Option<serde_json::Value>,
    ) -> Self {
        let reason = match reason.char_indices().nth(MAX_REASON_LEN) {
            None => SmolStr::new(reason),
            Some((cut, _)) => SmolStr::new(&reason[..cut]),
        };
        Self {
            at,
            kind,
            reason,
            detail: detail.map(Box::new),
        }
    }
}

/// A stored state change, returned by the `record_*` methods so the caller — who
/// knows the item's identity, which [`Stats`] does not — can log it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub from: ItemState,
    pub to: ItemState,
}

impl Transition {
    /// Emit the transition on [`STATE_LOG_TARGET`].
    ///
    /// Identity is on the event rather than a span: callers run outside the item's
    /// span (batch checks, listener annotate), and the exported stream has to be
    /// self-describing. `failure` is the one that caused a move into
    /// [`ItemState::Retrying`] or [`ItemState::Failing`].
    pub fn log(self, item_id: Uuid, item: &str, failure: Option<&Failure>) {
        info!(
            target: STATE_LOG_TARGET,
            item_id = %item_id,
            item = %item,
            from = %self.from,
            to = %self.to,
            failure_kind = failure.map(|f| <&'static str>::from(f.kind)),
            reason = failure.map(|f| f.reason.as_str()),
            "item state changed"
        );
    }
}

impl Stats {
    /// Record that a check is starting. `lag` is how late the scheduler was
    /// relative to the item's due time.
    ///
    /// Counted at the start rather than on completion, so that `total_failures`
    /// and `total_checks` share a denominator and an item wedged mid-check still
    /// shows a rising attempt count.
    pub fn record_attempt(&self, now_ms: i64, lag: Duration) {
        self.total_checks.fetch_add(1, Relaxed);
        self.last_attempt_ms.store(now_ms, Relaxed);
        self.last_lag_ms.store(as_millis_u32(lag), Relaxed);
    }

    /// Record a completed check. `records` is a **row** count — one per
    /// `MonitorLog`/`MonitorMetric` — not a data-point count.
    ///
    /// Returns the state change this caused, if any; see [`Transition`].
    pub fn record_success(
        &self,
        now_ms: i64,
        duration: Duration,
        records: u64,
    ) -> Option<Transition> {
        self.total_records.fetch_add(records, Relaxed);
        self.last_record_count
            .store(records.min(u32::MAX as u64) as u32, Relaxed);
        self.last_duration_ms
            .store(as_millis_u32(duration), Relaxed);
        self.last_success_ms.store(now_ms, Relaxed);
        if records > 0 {
            self.last_nonempty_ms.store(now_ms, Relaxed);
        }
        self.consecutive_failures.store(0, Relaxed);
        self.retries.store(0, Relaxed);
        self.mark_activity(now_ms, true);

        // Released last: see the ordering contract on `snapshot_at`.
        self.publish_state(if records > 0 {
            ItemState::Ok
        } else {
            ItemState::OkEmpty
        })
    }

    /// Record a failed check. `retrying` distinguishes "will try again shortly"
    /// from "gave up until the next scheduled run".
    ///
    /// Returns the state change this caused, if any; see [`Transition`].
    pub fn record_failure(
        &self,
        failure: Failure,
        duration: Duration,
        retries: u8,
        retrying: bool,
    ) -> Option<Transition> {
        self.total_failures.fetch_add(1, Relaxed);
        self.consecutive_failures.fetch_add(1, Relaxed);
        self.last_duration_ms
            .store(as_millis_u32(duration), Relaxed);
        self.retries.store(retries, Relaxed);
        // Read before `failure` is moved into the ring below. No `now_ms`
        // parameter is needed: `Failure::at` is already the event time.
        let at_ms = failure.at.timestamp_millis();

        // Newest first: that is the wire order `item_stats::to_proto` expects,
        // so prepend rather than append.
        let mut ring = match self.recent_failures.load_full() {
            Some(existing) => Vec::clone(&existing),
            None => Vec::with_capacity(MAX_RECENT_FAILURES),
        };
        ring.insert(0, failure.clone());
        ring.truncate(MAX_RECENT_FAILURES);
        self.recent_failures.store(Some(Arc::new(ring)));
        self.last_failure.store(Some(Arc::new(failure)));

        self.mark_activity(at_ms, false);

        self.publish_state(if retrying {
            ItemState::Retrying
        } else {
            ItemState::Failing
        })
    }

    /// Record rows that arrived for a listener item. `rows` is always ≥ 1 — a row
    /// had to match for the item to be resolved at all. Returns the state change
    /// this caused, if any; see [`Transition`].
    ///
    /// Deliberately not `record_attempt` + `record_success`: a listener item has no
    /// poll cycle, so the fields those two write are meaningless here.
    /// `last_duration_ms` would time the annotator rather than the target,
    /// `last_lag_ms` has no due time to be late against, and `retries` has nothing
    /// to retry. `total_checks` counts *receive events* (batches that matched), not
    /// checks — the ratio against `total_failures` still reads correctly.
    ///
    /// It does mark the [`ActivityWindow`], which is the one activity signal a
    /// listener item can meaningfully feed: a thousand batches inside one
    /// five-minute bucket set the same bit a thousand times, so a busy item can't
    /// flood it.
    pub fn record_ingest(&self, now_ms: i64, rows: u64) -> Option<Transition> {
        self.total_checks.fetch_add(1, Relaxed);
        self.total_records.fetch_add(rows, Relaxed);
        self.last_record_count
            .store(rows.min(u32::MAX as u64) as u32, Relaxed);
        self.last_attempt_ms.store(now_ms, Relaxed);
        self.last_success_ms.store(now_ms, Relaxed);
        if rows > 0 {
            self.last_nonempty_ms.store(now_ms, Relaxed);
        }
        self.consecutive_failures.store(0, Relaxed);
        self.mark_activity(now_ms, true);

        // Released last: see the ordering contract on `snapshot_at`.
        self.publish_state(if rows > 0 {
            ItemState::Ok
        } else {
            ItemState::OkEmpty
        })
    }

    /// Rows this item produced that never reached a sink.
    ///
    /// Two things land here. The batcher charges rows it collected but lost on the
    /// way downstream, and a collector charges rows it refused to ship — an SNMP
    /// walk row whose device answered too few of the configured oids to clear
    /// `max_unknown_oids_per_row`. Both are data the item gathered and the reader
    /// never got; the counter answers "how much went missing", not "why".
    ///
    /// Called from the batcher task while the worker that owns the item may
    /// still be running, which is why every counter here is read-modify-write.
    ///
    /// Counts row *copies* on the batcher path: a record is cloned once per processor
    /// attached to the item, so for a multi-processor item this can exceed
    /// `total_records`, and a drop on one processor does not mean the other processors
    /// missed the data. A collector-side discard is charged once, before the fan-out.
    ///
    /// Deliberately marks neither activity map. A drop is patrolli running out of
    /// downstream capacity or declining a partial answer, not this item's check
    /// failing — it doesn't move `state` either — so punching a hole in the sparkline
    /// would point the reader at the wrong device. An item whose every row is
    /// discarded therefore reads as [`ItemState::OkEmpty`], with `total_dropped` as
    /// the tell.
    pub fn add_dropped(&self, n: u64) {
        self.total_dropped.fetch_add(n, Relaxed);
    }

    /// Store the new state — always the last write of a `record_*` call, see the
    /// ordering contract on `snapshot_at` — and report whether it changed.
    ///
    /// A `swap` rather than load-then-store, so concurrent writers to one listener
    /// item each see the value they replaced and a change is reported exactly once.
    /// `Release` is all the ordering the reader relies on; nothing here depends on
    /// the value read back.
    fn publish_state(&self, to: ItemState) -> Option<Transition> {
        let from = ItemState::from(self.state.swap(to as u8, Release));
        (from != to).then_some(Transition { from, to })
    }

    /// Publish the next scheduled check, and the deadline past which the item
    /// reads as [`ItemState::Stale`]. Both are unix millis; 0 disables.
    pub fn set_next_check(&self, next_check_ms: i64, stale_after_ms: i64) {
        self.next_check_ms.store(next_check_ms, Relaxed);
        self.stale_after_ms.store(stale_after_ms, Relaxed);
    }

    /// Mark the 5-minute bucket `now_ms` falls in, on the `ok` or the `fail`
    /// plane of the windowed ring. See [`ActivityWindow`] for the encoding.
    ///
    /// Advancing the ring means recycling buckets: whoever moves `last_tick`
    /// forward first clears the span it skipped, because those buckets still hold
    /// results from a day ago and OR-ing fresh bits into them would fuse two runs
    /// into one. A gap of a full day means nothing in the ring is still in the
    /// window, so it all goes.
    ///
    /// A writer that arrives *behind* `last_tick` — a slightly lagging clock, or
    /// two workers reordered — skips the advance and just ORs. Its bucket is
    /// already inside the window, so it was cleared when the ring passed over it.
    ///
    /// **Race worth naming:** a concurrent writer can set a bit inside the span
    /// another writer is about to clear, and lose it. The window is nanoseconds
    /// wide and bounded to bits inside the span being recycled, `total_failures` /
    /// `last_failure` / `recent_failures` still record the event, and it is benign
    /// where it could actually happen: monitor items are single-writer by
    /// construction (`pop_due` holds the queue slot), and listener items only ever
    /// mark `ok`, so the worst case there is one lost bit on a bucket that the next
    /// batch sets again. Locking a diagnostics sparkline would cost more than it
    /// protects.
    fn mark_activity(&self, now_ms: i64, ok: bool) {
        let tick = activity_tick(now_ms);
        let w = &self.activity_window;
        let last = w.last_tick.load(Acquire);
        let gap = tick - last;
        if gap > 0
            && w.last_tick
                .compare_exchange(last, tick, AcqRel, Acquire)
                .is_ok()
        {
            // Not just a shortcut for the loop below — it is what keeps the *first*
            // write cheap. `last_tick` starts at 0 while a real timestamp is some
            // six million ticks, so without this every item's first write would
            // recycle bucket-by-bucket across the whole of unix history. Any gap of
            // a full lap covers all 288 buckets anyway, so the two are equivalent
            // in effect and wildly different in cost.
            if gap >= ACTIVITY_BUCKETS as i64 {
                for i in 0..ACTIVITY_WORDS {
                    w.ok[i].store(0, Relaxed);
                    w.fail[i].store(0, Relaxed);
                }
            } else {
                // Recycle (last, tick]: every bucket the ring just moved over.
                for t in (last + 1)..=tick {
                    let bucket = activity_bucket(t);
                    let mask = !(1u64 << (bucket % 64));
                    w.ok[bucket / 64].fetch_and(mask, Relaxed);
                    w.fail[bucket / 64].fetch_and(mask, Relaxed);
                }
            }
        }
        let bucket = activity_bucket(tick);
        let plane = if ok { &w.ok } else { &w.fail };
        plane[bucket / 64].fetch_or(1u64 << (bucket % 64), Relaxed);
    }

    /// Read every field into a [`StatsSnapshot`], deriving staleness against
    /// `now`.
    ///
    /// `state` is the synchronisation point. The writer publishes the failure
    /// pointers and counters with `Relaxed`, then stores `state` with
    /// `Release`; this reads `state` with `Acquire` **first**. That gives one
    /// invariant worth having: if a reader sees `Failing`, the `Failure` that
    /// explains it is already visible. Without it the UI can show a red dot
    /// next to a stale reason string, which is exactly the kind of thing that
    /// makes people stop trusting the page.
    ///
    /// Fields are still read one at a time, so a snapshot taken mid-write can
    /// mix one check's counters with the next one's. That's fine — the numbers
    /// are monotonic and the UI polls on a several-second cadence.
    ///
    /// Takes the clock as an argument so the staleness rule is testable without
    /// sleeping, and so a sweep over many items reads the clock once. Its offset is
    /// the one every timestamp in the snapshot comes back in, which is what keeps
    /// the cache itself free of any timezone: the caller decides, and the ring's
    /// bucket maths is absolute either way.
    pub fn snapshot_at(&self, now: DateTime<FixedOffset>) -> StatsSnapshot {
        let now_ms = now.timestamp_millis();
        let stored = ItemState::from(self.state.load(Acquire));
        // 0 is the "never happened" sentinel for every timestamp field. The
        // conversion is fallible only for millis beyond ±262,000 years, which no
        // clock this reads from can produce; treating that as "never" rather than
        // panicking keeps a corrupt-looking counter from taking down a stats sweep.
        let ts = |v: i64| {
            (v != 0)
                .then(|| DateTime::from_timestamp_millis(v))
                .flatten()
                .map(|t| t.with_timezone(&now.timezone()))
        };

        // Staleness is the one state no writer can set — an item is stale
        // precisely because nothing is executing it — so it is derived here.
        // `Failing` wins over `Stale`: a target that is actively erroring is the
        // more actionable fact, and `severity()` ranks it higher for the same
        // reason. `Unknown` is left alone; it means we couldn't read the byte.
        let stale_after = self.stale_after_ms.load(Relaxed);
        let state = match stored {
            ItemState::Failing | ItemState::Unknown => stored,
            _ if stale_after != 0 && now_ms > stale_after => ItemState::Stale,
            _ => stored,
        };

        // Gate on the *stored* state, not the derived one: an item that has
        // never run has no duration, lag or count to report even once it goes
        // stale, and a literal 0 would read as "instant, on time, empty".
        let ran = stored != ItemState::NeverRan;

        // Buckets after `last_tick` were never reached by a writer, so nothing
        // cleared them and they still hold results from a day ago. Mask them here:
        // an item that has gone silent must show an empty tail, not a day-old
        // strip that reads as current.
        let now_tick = activity_tick(now_ms);
        let (activity_ok, activity_fail) = self.activity_window.read_masked(now_tick);

        StatsSnapshot {
            state,
            total_checks: self.total_checks.load(Relaxed),
            total_failures: self.total_failures.load(Relaxed),
            total_records: self.total_records.load(Relaxed),
            total_dropped: self.total_dropped.load(Relaxed),
            consecutive_failures: self.consecutive_failures.load(Relaxed),
            retries: self.retries.load(Relaxed),

            last_attempt: ts(self.last_attempt_ms.load(Relaxed)),
            last_success: ts(self.last_success_ms.load(Relaxed)),
            last_nonempty: ts(self.last_nonempty_ms.load(Relaxed)),
            next_check: ts(self.next_check_ms.load(Relaxed)),

            last_duration_ms: ran.then(|| self.last_duration_ms.load(Relaxed)),
            last_lag_ms: ran.then(|| self.last_lag_ms.load(Relaxed)),
            last_record_count: ran.then(|| self.last_record_count.load(Relaxed)),

            activity_ok,
            activity_fail,
            // Floored by subtracting the offset into the current bucket, so the
            // result keeps `now`'s offset instead of being rebuilt from millis. The
            // fallback is unreachable for any real clock — 5 minutes before the
            // start of chrono's range — and a panic is fatal (`common::panic`),
            // which makes not risking it on a diagnostics read the cheap choice.
            activity_window_end: now
                .checked_sub_signed(TimeDelta::milliseconds(
                    now_ms.rem_euclid(ACTIVITY_BUCKET_MS),
                ))
                .unwrap_or(now),

            last_failure: self.last_failure.load_full(),
            recent_failures: self.recent_failures.load_full(),
        }
    }

    /// [`Stats::snapshot_at`] against the current wall clock, in the process
    /// timezone — the same offset [`Failure::at`] is stamped with.
    pub fn snapshot(&self) -> StatsSnapshot {
        self.snapshot_at(common::now().fixed_offset())
    }
}

/// One item in the cache, and — cloned by [`Item::pop_due`] — the unit of work a
/// monitor worker executes.
///
/// A clone is a **detached snapshot**, not a handle on the cached entry: `item`
/// and `stats` are shared through their `Arc`s, but `monitor` is `Copy`, so
/// mutating a clone's `monitor` changes nothing in the cache. The only way to
/// publish an updated monitor is [`Item::requeue`] — forgetting that is what
/// froze `retries` at 0 before, so if you find yourself editing `monitor` on a
/// popped entry, check that a `requeue` follows.
#[derive(Clone)]
pub struct Entry {
    pub item: Arc<item::Item>,
    pub monitor: item::ItemMonitor,
    pub stats: Arc<Stats>,
    /// Which reload pass last saw this entry — see [`Item::begin_reload`].
    /// Meaningless on a detached clone, where it is just the value at clone time.
    generation: u64,
}

impl Entry {
    pub fn new(item: item::Item) -> Self {
        let monitor = item::ItemMonitor::new(item.id, &item.schedule);
        let stats = Arc::new(Stats::default());
        Self {
            item: Arc::new(item),
            monitor,
            stats,
            generation: 0,
        }
    }
}

/// A monotonic deadline expressed as unix millis, for a field the API has to
/// render as a wall-clock time. A deadline already in the past reads as "now".
fn wall_clock_ms(deadline: Instant) -> i64 {
    let ahead = deadline.saturating_duration_since(Instant::now());
    common::now().timestamp_millis() + ahead.as_millis() as i64
}

pub struct Item {
    data: FxHashMap<Uuid, Entry>,
    queue: PriorityQueue<Uuid, Reverse<Instant>, FxBuildHasher>,
    last_compact: Instant,
    /// Bumped by [`Item::begin_reload`]; entries not re-stamped by the time
    /// [`Item::end_reload`] runs are dropped.
    generation: u64,
}

impl Default for Item {
    fn default() -> Self {
        Self::new()
    }
}

impl Item {
    pub fn new() -> Self {
        Self {
            data: FxHashMap::default(),
            queue: PriorityQueue::default(),
            last_compact: Instant::now(),
            generation: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn replace(&mut self, items: Vec<item::Item>) {
        self.begin_reload();
        for item in items {
            self.insert(item);
        }
        self.end_reload();
    }

    /// Start a full reload. Entries stay alive until [`Item::end_reload`], so an
    /// item re-loaded unchanged keeps its [`Stats`].
    ///
    /// This exists instead of `clear()` because a full reload happens on every
    /// node-lock flap, not just at startup — wiping every item's health history
    /// on a transient database blip would make the stats page useless.
    pub fn begin_reload(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Finish a full reload: drop every entry the reload didn't re-insert.
    ///
    /// Only call this once the load has fully succeeded. Bailing out midway and
    /// sweeping anyway would delete items that simply hadn't been read yet.
    pub fn end_reload(&mut self) {
        let current = self.generation;
        let stale: Vec<Uuid> = self
            .data
            .iter()
            .filter(|(_, entry)| entry.generation != current)
            .map(|(id, _)| *id)
            .collect();
        let removed = stale.len();
        for id in stale {
            self.data.remove(&id);
            self.queue.remove(&id);
        }
        self.compact();
        debug!(
            removed,
            remaining = self.data.len(),
            "item cache reload swept stale items"
        );
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    fn compact(&mut self) {
        if self.last_compact.elapsed() > Duration::from_mins(5) {
            self.data.shrink_to_fit();
            self.queue.shrink_to_fit();
        }
        self.last_compact = Instant::now();
    }

    /// Insert an item, or update one already cached — preserving its [`Stats`].
    ///
    /// Preserving matters because config reload re-inserts unconditionally: the
    /// `log_endpoint_cache_operation` trigger cascades an item upsert to every
    /// item on an endpoint, byte-identical rows included, so bumping one SNMP
    /// community would otherwise erase the health history of every item on that
    /// device.
    ///
    /// The existing [`item::ItemMonitor`] is kept too, so an interval item's
    /// phase isn't reset on every unrelated edit; `requeue` recomputes
    /// `next_check` from the new schedule on the next cycle anyway.
    pub fn insert(&mut self, item: item::Item) {
        let id = item.id;
        let schedulable = item.schedulable();
        let generation = self.generation;

        let Some(entry) = self.data.get_mut(&id) else {
            let mut entry = Entry::new(item);
            entry.generation = generation;
            let next_check = entry.monitor.next_check;
            self.data.insert(id, entry);
            if schedulable {
                self.queue.push(id, Reverse(next_check));
            }
            return;
        };

        let was_schedulable = entry.item.schedulable();
        entry.item = Arc::new(item);
        entry.generation = generation;
        let next_check = entry.monitor.next_check;

        // Deliberately *not* re-queuing an item that is already known: it may be
        // in flight right now (`pop_due` took its queue slot and the worker will
        // `requeue` it), and pushing here would let the scheduler pop it a second
        // time and run the same item on two workers at once. Only the
        // schedulability transitions need the queue touched.
        match (was_schedulable, schedulable) {
            (false, true) => {
                self.queue.push(id, Reverse(next_check));
            }
            (true, false) => {
                self.queue.remove(&id);
            }
            _ => {}
        }
    }

    pub fn remove(&mut self, id: Uuid) -> Option<Arc<item::Item>> {
        if let Some(entry) = self.data.remove(&id) {
            self.queue.remove(&id);
            self.compact();
            return Some(entry.item);
        }
        None
    }

    pub fn execute_now(&mut self, id: &Uuid) -> Result<()> {
        let Some(entry) = self.data.get_mut(id) else {
            return Err(anyhow::anyhow!("item not found"));
        };
        if !entry.item.schedulable() {
            return Err(anyhow::anyhow!("item not schedulable"));
        }
        let now = Instant::now();
        if self.queue.change_priority(id, Reverse(now)).is_none() {
            return Err(anyhow::anyhow!("item is already running"));
        }
        entry.monitor.next_check = now;
        Ok(())
    }

    pub fn get(&self, id: &Uuid) -> Option<&Arc<item::Item>> {
        if let Some(entry) = self.data.get(id) {
            return Some(&entry.item);
        }
        None
    }

    /// The health counters for one item.
    ///
    /// Returns the `Arc` rather than a snapshot so a caller can read it after
    /// dropping the cache lock — a stats sweep must not hold the lock while it
    /// serialises.
    pub fn stats(&self, id: &Uuid) -> Option<Arc<Stats>> {
        self.data.get(id).map(|entry| entry.stats.clone())
    }

    /// Return an item to the queue after a check, and publish when its next one
    /// is due.
    pub fn requeue(&mut self, mut monitor: item::ItemMonitor) {
        let id = monitor.id;
        let Some(entry) = self.data.get_mut(&id) else {
            trace!(item_id = %id, "requeue of removed item ignored");
            return;
        };

        if monitor.is_due() {
            monitor.update_next_check(&entry.item.schedule);
        }
        entry.monitor = monitor;
        if entry.item.schedulable() {
            // The stale grace has to exceed the item's own timeout, or a
            // legitimately slow check — an SNMP walk with a 30s budget — would read
            // as stale just for being slow.
            let next_check_ms = wall_clock_ms(monitor.next_check);
            let grace = (entry.item.timeout * 2).max(Duration::from_secs(60));
            entry
                .stats
                .set_next_check(next_check_ms, next_check_ms + grace.as_millis() as i64);
            self.queue.push(id, Reverse(monitor.next_check));
        } else {
            // Became unschedulable mid-flight. Publishing a next check it will
            // never honour would make it read as stale forever, so report "not
            // scheduled" instead.
            entry.stats.set_next_check(0, 0);
        }
    }

    /// Take every item whose next check has come due.
    ///
    /// The entry stays in `data` and only its queue slot is taken, which is what
    /// makes a monitor item single-writer: nothing can hand the same item out
    /// again until [`Item::requeue`] puts the slot back.
    ///
    /// Each returned [`Entry`] is a detached clone — the `Arc`s are shared, the
    /// monitor is a copy. See the note on `Entry`.
    pub fn pop_due(&mut self) -> Vec<Entry> {
        let mut items = Vec::new();
        while let Some(entry) = self
            .queue
            .pop_if(|_, Reverse(when)| *when <= Instant::now())
            .map(|(id, _)| self.data.get(&id).unwrap())
        {
            items.push(entry.clone());
        }
        self.compact();
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::item::{Config, Dummy, DummyPayload, Item as ItemConfig, Schedule};
    use std::time::Duration as StdDuration;

    fn dummy_item(schedule: Schedule) -> ItemConfig {
        ItemConfig::new(
            Uuid::now_v7(),
            "test".into(),
            None,
            Uuid::now_v7(),
            schedule,
            None,
            StdDuration::from_secs(1),
            vec![],
            Config::Dummy(Dummy {
                payload: DummyPayload::default(),
                count: 1,
            }),
        )
    }

    #[test]
    fn pop_due_empty_cache_returns_empty() {
        let mut cache = Item::new();
        assert!(cache.pop_due().is_empty());
    }

    #[test]
    fn pop_due_returns_empty_when_no_item_is_due() {
        let mut cache = Item::new();
        // Schedule::Never sets next_check ~1y out, so nothing should be due.
        cache.insert(dummy_item(Schedule::Never));
        cache.insert(dummy_item(Schedule::Never));
        assert!(cache.pop_due().is_empty());
    }

    #[test]
    fn pop_due_returns_only_items_whose_next_check_has_elapsed() {
        let mut cache = Item::new();
        let due = dummy_item(Schedule::Never);
        let due_id = due.id;
        let pending = dummy_item(Schedule::Never);
        let pending_id = pending.id;

        cache.insert(due);
        cache.insert(pending);

        // execute_now resets one item's next_check to "now" — it should pop,
        // while the other stays scheduled in the far future.
        cache.execute_now(&due_id).unwrap();

        let popped = cache.pop_due();
        assert_eq!(popped.len(), 1);
        assert_eq!(popped[0].item.id, due_id);
        assert!(cache.get(&pending_id).is_some());
    }

    #[test]
    fn pop_due_drains_all_due_items_in_one_call() {
        let mut cache = Item::new();
        let mut ids = Vec::new();
        for _ in 0..5 {
            let item = dummy_item(Schedule::Never);
            let id = item.id;
            cache.insert(item);
            cache.execute_now(&id).unwrap();
            ids.push(id);
        }

        let popped = cache.pop_due();
        assert_eq!(popped.len(), ids.len());
        for due in &popped {
            assert!(ids.contains(&due.item.id));
        }
        // Items aren't requeued automatically; queue is now drained.
        assert!(cache.pop_due().is_empty());
    }

    #[test]
    fn pop_due_returns_items_in_ascending_due_order() {
        let mut cache = Item::new();
        let first = dummy_item(Schedule::Never);
        let first_id = first.id;
        let second = dummy_item(Schedule::Never);
        let second_id = second.id;
        let third = dummy_item(Schedule::Never);
        let third_id = third.id;

        cache.insert(first);
        cache.insert(second);
        cache.insert(third);

        // Stamp due-times in a deliberate, non-insertion order.
        cache.execute_now(&second_id).unwrap();
        std::thread::sleep(StdDuration::from_millis(5));
        cache.execute_now(&first_id).unwrap();
        std::thread::sleep(StdDuration::from_millis(5));
        cache.execute_now(&third_id).unwrap();

        let popped = cache.pop_due();
        let ordered: Vec<Uuid> = popped.iter().map(|due| due.item.id).collect();
        assert_eq!(ordered, vec![second_id, first_id, third_id]);
    }

    #[test]
    fn pop_due_returns_item_again_after_requeue() {
        let mut cache = Item::new();
        let item = dummy_item(Schedule::Never);
        let id = item.id;
        cache.insert(item);
        cache.execute_now(&id).unwrap();

        let popped = cache.pop_due();
        assert_eq!(popped.len(), 1);
        let monitor = popped[0].monitor;
        assert!(cache.pop_due().is_empty());

        // Requeue with the (now-past) monitor — it's "due", so the cache
        // refreshes next_check from the schedule (Never => far future)
        // before pushing it back onto the queue.
        cache.requeue(monitor);
        assert!(cache.pop_due().is_empty());
        assert!(cache.get(&id).is_some());
    }

    #[test]
    fn pop_due_ignores_removed_items() {
        let mut cache = Item::new();
        let item = dummy_item(Schedule::Never);
        let id = item.id;
        cache.insert(item);
        cache.execute_now(&id).unwrap();

        assert!(cache.remove(id).is_some());
        // The queue entry is dropped alongside the data entry.
        assert!(cache.pop_due().is_empty());
    }

    fn failure(kind: FailureKind, reason: &str) -> Failure {
        Failure::new(common::now().fixed_offset(), kind, reason, None)
    }

    /// The offset every test reads snapshots in. Deliberately **not** UTC: bucket
    /// positions and staleness are absolute, so nothing about them may move with the
    /// caller's offset — only the offset the timestamps come back rendered in.
    const READ_OFFSET_SECS: i32 = 7 * 3600;

    /// Unix millis as the instant [`Stats::snapshot_at`] takes. Writes still speak
    /// millis, so a test that pins a moment uses the same number for both.
    fn clock(ms: i64) -> DateTime<FixedOffset> {
        DateTime::from_timestamp_millis(ms)
            .expect("test timestamp in range")
            .with_timezone(&FixedOffset::east_opt(READ_OFFSET_SECS).expect("valid offset"))
    }

    /// The regression guard for `requeue` writing the monitor back into the
    /// entry. Without it `retries` resets to the insert-time 0 on every pop, so
    /// `MAX_RETRIES` is unreachable and a broken item retries forever.
    #[test]
    fn requeue_writes_the_updated_monitor_back() {
        let mut cache = Item::new();
        let item = dummy_item(Schedule::Never);
        let id = item.id;
        cache.insert(item);
        cache.execute_now(&id).unwrap();

        let mut monitor = cache.pop_due()[0].monitor;
        assert_eq!(monitor.retries, 0);
        monitor.retries = 2;
        cache.requeue(monitor);

        cache.execute_now(&id).unwrap();
        assert_eq!(cache.pop_due()[0].monitor.retries, 2);
    }

    #[test]
    fn requeue_publishes_next_check_and_the_stale_deadline() {
        let mut cache = Item::new();
        let item = dummy_item(Schedule::Interval(StdDuration::from_secs(60)));
        let id = item.id;
        cache.insert(item);
        cache.execute_now(&id).unwrap();

        let due = cache.pop_due().pop().unwrap();
        due.stats.record_success(1_000, StdDuration::ZERO, 1);
        cache.requeue(due.monitor);

        let stats = cache.stats(&id).unwrap();
        let next = stats
            .snapshot()
            .next_check
            .expect("next check published")
            .timestamp_millis();

        // dummy_item has a 1s timeout, so the grace floor of 60s applies.
        assert_eq!(stats.snapshot_at(clock(next)).state, ItemState::Ok);
        assert_eq!(stats.snapshot_at(clock(next + 60_000)).state, ItemState::Ok);
        assert_eq!(
            stats.snapshot_at(clock(next + 60_001)).state,
            ItemState::Stale
        );
    }

    /// An endpoint edit cascades an item upsert to every item on that endpoint, so
    /// re-inserting must not reset the health history.
    #[test]
    fn insert_preserves_stats_of_an_existing_item() {
        let mut cache = Item::new();
        let item = dummy_item(Schedule::Never);
        let id = item.id;
        cache.insert(item.clone());

        let before = cache.stats(&id).unwrap();
        before.record_attempt(1_000, StdDuration::ZERO);
        before.record_success(1_000, StdDuration::from_millis(5), 3);

        cache.insert(item);

        let after = cache.stats(&id).unwrap();
        assert!(Arc::ptr_eq(&before, &after), "stats must be the same Arc");
        let snap = after.snapshot_at(clock(1_000));
        assert_eq!(snap.total_checks, 1);
        assert_eq!(snap.total_records, 3);
        assert_eq!(snap.state, ItemState::Ok);
    }

    /// Re-inserting an in-flight item must not put it back on the queue: the
    /// worker still holds it and will requeue it itself, so pushing here would let
    /// the scheduler run the same item on two workers at once.
    #[test]
    fn insert_does_not_requeue_an_in_flight_item() {
        let mut cache = Item::new();
        let item = dummy_item(Schedule::Never);
        let id = item.id;
        cache.insert(item.clone());
        cache.execute_now(&id).unwrap();
        assert_eq!(cache.pop_due().len(), 1);

        cache.insert(item);
        assert!(cache.pop_due().is_empty());
    }

    /// The same double-schedule hazard as above, from the other direction: an
    /// "execute now" arriving while the item is mid-check must not hand it to a
    /// second worker.
    #[test]
    fn execute_now_refuses_an_in_flight_item() {
        let mut cache = Item::new();
        let item = dummy_item(Schedule::Never);
        let id = item.id;
        cache.insert(item);

        // Idle: the reschedule lands and the item pops.
        cache.execute_now(&id).expect("idle item reschedules");
        let popped = cache.pop_due();
        assert_eq!(popped.len(), 1);

        // In flight — `pop_due` holds its queue slot until `requeue`.
        let err = cache
            .execute_now(&id)
            .expect_err("in-flight item is refused");
        assert!(err.to_string().contains("already running"), "{err}");
        assert!(
            cache.pop_due().is_empty(),
            "a refused execute_now must not leave a second copy queued"
        );

        // Once the worker hands it back, it is schedulable again.
        cache.requeue(popped[0].monitor);
        cache.execute_now(&id).expect("requeued item reschedules");
        assert_eq!(cache.pop_due().len(), 1);
    }

    #[test]
    fn execute_now_rejects_a_non_schedulable_item() {
        let mut cache = Item::new();
        let mut item = dummy_item(Schedule::Never);
        item.config = Config::Listener(item::Listener {
            match_strategy: item::ListenerItemMatchStrategy::None,
        });
        let id = item.id;
        cache.insert(item);

        let err = cache
            .execute_now(&id)
            .expect_err("listener items don't run");
        assert!(err.to_string().contains("not schedulable"), "{err}");
    }

    #[test]
    fn reload_keeps_re_seen_items_and_drops_the_rest() {
        let mut cache = Item::new();
        let kept = dummy_item(Schedule::Never);
        let kept_id = kept.id;
        let gone = dummy_item(Schedule::Never);
        let gone_id = gone.id;
        cache.insert(kept.clone());
        cache.insert(gone);
        cache
            .stats(&kept_id)
            .unwrap()
            .record_attempt(1_000, StdDuration::ZERO);

        cache.begin_reload();
        cache.insert(kept);
        cache.end_reload();

        assert!(cache.get(&kept_id).is_some());
        assert!(cache.get(&gone_id).is_none());
        assert_eq!(
            cache
                .stats(&kept_id)
                .unwrap()
                .snapshot_at(clock(1_000))
                .total_checks,
            1,
            "a reload must not wipe an item it re-loaded unchanged"
        );
    }

    #[test]
    fn record_success_distinguishes_empty_from_productive() {
        let stats = Stats::default();
        stats.record_attempt(1_000, StdDuration::from_millis(20));
        stats.record_success(1_000, StdDuration::from_millis(7), 4);

        let snap = stats.snapshot_at(clock(1_000));
        assert_eq!(snap.state, ItemState::Ok);
        assert_eq!(snap.total_records, 4);
        assert_eq!(snap.last_record_count, Some(4));
        assert_eq!(snap.last_duration_ms, Some(7));
        assert_eq!(snap.last_lag_ms, Some(20));
        assert_eq!(snap.last_nonempty, Some(clock(1_000)));

        let empty = Stats::default();
        empty.record_attempt(1_000, StdDuration::ZERO);
        empty.record_success(1_000, StdDuration::ZERO, 0);
        let snap = empty.snapshot_at(clock(1_000));
        assert_eq!(snap.state, ItemState::OkEmpty);
        // The check ran and returned nothing — that is a real 0, not "unknown".
        assert_eq!(snap.last_record_count, Some(0));
        assert_eq!(snap.last_nonempty, None);
    }

    #[test]
    fn record_failure_tracks_state_streak_and_ring_order() {
        let stats = Stats::default();
        stats.record_failure(
            failure(FailureKind::Timeout, "first"),
            StdDuration::from_millis(3),
            1,
            true,
        );
        assert_eq!(stats.snapshot_at(clock(0)).state, ItemState::Retrying);

        stats.record_failure(
            failure(FailureKind::Auth, "second"),
            StdDuration::from_millis(4),
            0,
            false,
        );
        let snap = stats.snapshot_at(clock(0));
        assert_eq!(snap.state, ItemState::Failing);
        assert_eq!(snap.total_failures, 2);
        assert_eq!(snap.consecutive_failures, 2);
        assert_eq!(snap.last_failure.as_ref().unwrap().reason, "second");

        // Newest first — the order `item_stats::to_proto` puts on the wire.
        let ring = snap.recent_failures.as_ref().unwrap();
        assert_eq!(ring[0].reason, "second");
        assert_eq!(ring[1].reason, "first");

        stats.record_success(2_000, StdDuration::ZERO, 1);
        let snap = stats.snapshot_at(clock(0));
        assert_eq!(snap.consecutive_failures, 0);
        assert_eq!(snap.total_failures, 2, "the total is cumulative");
        // The explanation of the last failure survives a recovery.
        assert!(snap.last_failure.is_some());
    }

    #[test]
    fn recent_failures_ring_is_capped() {
        let stats = Stats::default();
        for i in 0..MAX_RECENT_FAILURES + 5 {
            stats.record_failure(
                failure(FailureKind::Timeout, &format!("f{i}")),
                StdDuration::ZERO,
                0,
                false,
            );
        }
        let snap = stats.snapshot_at(clock(0));
        let ring = snap.recent_failures.as_ref().unwrap();
        assert_eq!(ring.len(), MAX_RECENT_FAILURES);
        assert_eq!(ring[0].reason, "f14");
    }

    // --- windowed activity ring ---
    //
    // No timezone anywhere: buckets are absolute 5-minute ticks mod 288, so a
    // timestamp is just arithmetic. `DAY` is one full lap of the ring.

    const BUCKET: i64 = ACTIVITY_BUCKET_MS;
    const DAY: i64 = ACTIVITY_BUCKETS as i64 * BUCKET;

    /// Unix millis for bucket `n` of the lap starting at `DAY * 100_000`, offset
    /// far from the epoch so nothing accidentally passes on a zero.
    fn at_bucket(n: i64) -> i64 {
        DAY * 100_000 + n * BUCKET
    }

    fn failure_at(ms: i64, kind: FailureKind) -> Failure {
        Failure::new(clock(ms), kind, "x", None)
    }

    /// Which buckets a plane has set, so assertions read as a list of positions
    /// rather than a hex mask. `get` rather than indexing, so an empty plane — the
    /// form an idle item ships — reads as "no buckets" instead of panicking.
    fn buckets_set(plane: &Bytes) -> Vec<usize> {
        (0..ACTIVITY_BUCKETS)
            .filter(|i| plane.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0))
            .collect()
    }

    #[test]
    fn bucket_zero_is_utc_midnight() {
        // The epoch is UTC-midnight aligned and a lap is exactly 24 h, so a bit
        // position is a fixed UTC time of day with no timezone maths involved.
        assert_eq!(activity_bucket(activity_tick(0)), 0);
        assert_eq!(activity_bucket(activity_tick(12 * 3_600_000)), 144);
        assert_eq!(activity_bucket(activity_tick(DAY - 1)), 287);
        assert_eq!(activity_bucket(activity_tick(DAY)), 0);
        // Pre-epoch instants floor rather than truncating toward zero.
        assert_eq!(activity_bucket(activity_tick(-1)), 287);
    }

    /// The reason the window is two planes and not one bit per bucket: a bucket
    /// the item wasn't due in must be distinguishable from one where it failed.
    #[test]
    fn a_bucket_with_no_result_is_clear_in_both_planes() {
        let stats = Stats::default();
        stats.record_success(at_bucket(144), StdDuration::ZERO, 1);

        let snap = stats.snapshot_at(clock(at_bucket(144)));
        assert_eq!(buckets_set(&snap.activity_ok), vec![144]);
        assert_eq!(buckets_set(&snap.activity_fail), Vec::<usize>::new());
        assert_eq!(snap.activity_window_end, clock(at_bucket(144)));
    }

    #[test]
    fn success_and_failure_in_one_bucket_set_both_planes() {
        let at = at_bucket(2);
        let stats = Stats::default();
        stats.record_success(at, StdDuration::ZERO, 1);
        stats.record_failure(
            failure_at(at, FailureKind::Timeout),
            StdDuration::ZERO,
            0,
            false,
        );

        let snap = stats.snapshot_at(clock(at));
        // Both bits on bucket 2 — the "flapped inside five minutes" state that a
        // single-plane map cannot express.
        assert_eq!(buckets_set(&snap.activity_ok), vec![2]);
        assert_eq!(buckets_set(&snap.activity_fail), vec![2]);
    }

    #[test]
    fn repeated_results_in_one_bucket_set_one_bit() {
        let at = at_bucket(102);
        let stats = Stats::default();
        for _ in 0..50 {
            stats.record_ingest(at, 3);
        }
        let snap = stats.snapshot_at(clock(at));
        assert_eq!(buckets_set(&snap.activity_ok), vec![102]);
    }

    /// The bit layout a consumer decodes against: 36 bytes, LSB-first, bucket `i` at
    /// bit `i % 8` of byte `i / 8`, and no room for the padding the internal words
    /// carry.
    #[test]
    fn a_plane_is_thirty_six_bytes_lsb_first() {
        let stats = Stats::default();
        // 00:00–00:05, the first bucket of the day.
        stats.record_success(at_bucket(0), StdDuration::ZERO, 1);
        // 12:00, halfway: byte 18, bit 0.
        stats.record_success(at_bucket(144), StdDuration::ZERO, 1);
        // 23:55, the very last bit of the plane.
        stats.record_success(at_bucket(287), StdDuration::ZERO, 1);

        let plane = stats.snapshot_at(clock(at_bucket(287))).activity_ok;
        assert_eq!(plane.len(), ACTIVITY_PLANE_BYTES);
        assert_eq!(plane.len(), 36);
        assert_eq!(plane[0], 0b0000_0001);
        assert_eq!(plane[18], 0b0000_0001);
        assert_eq!(plane[35], 0b1000_0000);
        // Buckets 288..320 exist as padding in the last internal word; packing must
        // not carry them out, or a consumer would decode 320 buckets from a 24-hour
        // window.
        assert_eq!(buckets_set(&plane), vec![0, 144, 287]);
    }

    /// The window end names the newest *bucket*, not the instant of the read, so a
    /// consumer can label bucket `i` without knowing when the snapshot was taken.
    #[test]
    fn the_window_end_is_the_start_of_the_newest_bucket() {
        let stats = Stats::default();
        // The last millisecond of the bucket: as far from its start as a read gets.
        let read = at_bucket(144) + BUCKET - 1;
        stats.record_success(read, StdDuration::ZERO, 1);

        let snap = stats.snapshot_at(clock(read));
        assert_eq!(snap.activity_window_end, clock(at_bucket(144)));
        assert_eq!(buckets_set(&snap.activity_ok), vec![144]);
    }

    /// A plane with nothing in it ships as no bytes at all rather than 36 zero
    /// bytes, so a stats read over many quiet items allocates nothing — and an empty
    /// plane still decodes as "no buckets set".
    ///
    /// Both ways in, because they are different code paths: an item that has never
    /// reported never reaches the packer, while a healthy item's `fail` plane goes
    /// through it with every bit clear.
    #[test]
    fn a_plane_with_nothing_in_it_ships_as_no_bytes() {
        let never = Stats::default().snapshot_at(clock(at_bucket(144)));
        assert!(never.activity_ok.is_empty());
        assert!(never.activity_fail.is_empty());
        assert_eq!(buckets_set(&never.activity_ok), Vec::<usize>::new());

        let healthy = Stats::default();
        healthy.record_success(at_bucket(144), StdDuration::ZERO, 1);
        let snap = healthy.snapshot_at(clock(at_bucket(144)));
        assert_eq!(snap.activity_ok.len(), ACTIVITY_PLANE_BYTES);
        assert!(
            snap.activity_fail.is_empty(),
            "an item that has never failed carries no fail plane"
        );
    }

    /// Bucket positions are absolute, so the offset a caller reads in changes only
    /// how timestamps are rendered — never which bucket a result landed in.
    #[test]
    fn the_read_offset_moves_no_buckets() {
        let stats = Stats::default();
        stats.record_success(at_bucket(144), StdDuration::ZERO, 1);

        let utc = stats.snapshot_at(clock(at_bucket(144)).to_utc().fixed_offset());
        let east = stats.snapshot_at(clock(at_bucket(144)));
        assert_eq!(utc.activity_ok, east.activity_ok);
        assert_eq!(utc.activity_window_end, east.activity_window_end);
        // Same instant, reported in the offset each caller asked for.
        assert_eq!(utc.activity_window_end.offset().local_minus_utc(), 0);
        assert_eq!(
            east.activity_window_end.offset().local_minus_utc(),
            READ_OFFSET_SECS
        );
        assert_eq!(
            east.last_success
                .expect("succeeded")
                .offset()
                .local_minus_utc(),
            READ_OFFSET_SECS,
            "every timestamp in a snapshot carries the caller's offset"
        );
    }

    /// The point of the ring: crossing midnight recycles one bucket, not the whole
    /// map, so yesterday's later hours survive into today.
    #[test]
    fn crossing_midnight_keeps_the_rest_of_the_day() {
        let stats = Stats::default();
        // Yesterday at 23:50 and 23:55, then today at 00:00.
        stats.record_success(at_bucket(286), StdDuration::ZERO, 1);
        stats.record_success(at_bucket(287), StdDuration::ZERO, 1);
        stats.record_success(at_bucket(288), StdDuration::ZERO, 1);

        let snap = stats.snapshot_at(clock(at_bucket(288)));
        // Bucket 0 is today's 00:00; 286 and 287 are still yesterday's evening.
        assert_eq!(buckets_set(&snap.activity_ok), vec![0, 286, 287]);
        assert_eq!(snap.activity_window_end, clock(at_bucket(288)));
    }

    /// A bucket being reused a lap later must not merge the two runs — the failure
    /// mode a position-within-the-day marker would have had.
    #[test]
    fn a_recycled_bucket_does_not_merge_with_a_lap_ago() {
        let stats = Stats::default();
        // A daily item: succeeds at bucket 36, then fails at the same bucket a
        // full lap later.
        stats.record_success(at_bucket(36), StdDuration::ZERO, 1);
        stats.record_failure(
            failure_at(at_bucket(36 + 288), FailureKind::Timeout),
            StdDuration::ZERO,
            0,
            false,
        );

        let snap = stats.snapshot_at(clock(at_bucket(36 + 288)));
        // Only the failure. Yesterday's success must not still be sitting there,
        // or the bucket would read as flapping forever.
        assert_eq!(buckets_set(&snap.activity_ok), Vec::<usize>::new());
        assert_eq!(buckets_set(&snap.activity_fail), vec![36]);
    }

    /// Advancing must recycle the buckets it steps over, because those hold results
    /// from the previous lap. Without it a bucket that has fallen out of the window
    /// keeps showing a day-old result forever.
    #[test]
    fn advancing_recycles_stale_buckets_it_steps_over() {
        let stats = Stats::default();
        // Lap 0: buckets 200 and 250.
        stats.record_success(at_bucket(200), StdDuration::ZERO, 1);
        stats.record_success(at_bucket(250), StdDuration::ZERO, 1);
        // Lap 1, bucket 210. The gap from 250 is 248 — under a full lap, so this
        // takes the recycling path and steps over buckets 251..287 then 0..210,
        // which includes bucket 200's lap-0 result.
        stats.record_success(at_bucket(288 + 210), StdDuration::ZERO, 1);

        let snap = stats.snapshot_at(clock(at_bucket(288 + 210)));
        // 200 is gone: its tick fell out of the 288-bucket window. 250 stays,
        // because its lap-0 tick is still inside it — recycling must not overreach.
        assert_eq!(buckets_set(&snap.activity_ok), vec![210, 250]);
    }

    #[test]
    fn advancing_leaves_untouched_buckets_alone() {
        let stats = Stats::default();
        for b in [10, 11, 12] {
            stats.record_success(at_bucket(b), StdDuration::ZERO, 1);
        }
        // Jump forward 4 buckets: 13..15 are skipped and 16 is written.
        stats.record_success(at_bucket(16), StdDuration::ZERO, 1);

        let snap = stats.snapshot_at(clock(at_bucket(16)));
        assert_eq!(buckets_set(&snap.activity_ok), vec![10, 11, 12, 16]);
    }

    #[test]
    fn a_gap_of_a_full_lap_clears_everything() {
        let stats = Stats::default();
        for b in [10, 100, 200] {
            stats.record_success(at_bucket(b), StdDuration::ZERO, 1);
        }
        // Silent for a full lap: nothing in the ring is still inside the window.
        stats.record_success(at_bucket(200 + 288), StdDuration::ZERO, 1);

        let snap = stats.snapshot_at(clock(at_bucket(200 + 288)));
        assert_eq!(buckets_set(&snap.activity_ok), vec![200]);
    }

    /// A writer arriving behind `last_tick` still records, and must not recycle
    /// anything — its bucket is already inside the window.
    #[test]
    fn a_writer_behind_the_newest_bucket_records_without_recycling() {
        let stats = Stats::default();
        stats.record_success(at_bucket(50), StdDuration::ZERO, 1);
        stats.record_success(at_bucket(48), StdDuration::ZERO, 1);

        let snap = stats.snapshot_at(clock(at_bucket(50)));
        assert_eq!(buckets_set(&snap.activity_ok), vec![48, 50]);
    }

    /// Nothing clears while an item is silent, so the reader masks the tail. A
    /// dead item must not show a day-old strip that reads as current.
    #[test]
    fn the_reader_masks_buckets_since_the_last_write() {
        let stats = Stats::default();
        stats.record_success(at_bucket(100), StdDuration::ZERO, 1);
        stats.record_success(at_bucket(200), StdDuration::ZERO, 1);

        // Read at the last write: nothing has aged out yet.
        let snap = stats.snapshot_at(clock(at_bucket(200)));
        assert_eq!(buckets_set(&snap.activity_ok), vec![100, 200]);

        // Read 250 buckets later with no writes in between. The window is now the
        // 288 buckets ending there, and bucket 100's stored result sits at a tick
        // that has fallen outside it — nothing cleared it, because clearing only
        // happens on a write, so the reader must mask it. Bucket 200 is still in.
        let snap = stats.snapshot_at(clock(at_bucket(200 + 250)));
        assert_eq!(buckets_set(&snap.activity_ok), vec![200]);

        // A full lap later: everything is out of the window.
        let snap = stats.snapshot_at(clock(at_bucket(200 + 288)));
        assert_eq!(buckets_set(&snap.activity_ok), Vec::<usize>::new());
        assert_eq!(buckets_set(&snap.activity_fail), Vec::<usize>::new());
        // The counters are cumulative and stay put; only the window is scoped.
        assert_eq!(snap.total_records, 2);
    }

    /// A listener item has no poll cycle, but it still lights its bucket — the
    /// windowed ring is the one activity signal it can feed.
    #[test]
    fn record_ingest_marks_the_window() {
        let at = at_bucket(72);
        let stats = Stats::default();
        stats.record_ingest(at, 9);

        let snap = stats.snapshot_at(clock(at));
        assert_eq!(buckets_set(&snap.activity_ok), vec![72]);
        assert_eq!(buckets_set(&snap.activity_fail), Vec::<usize>::new());
    }

    #[test]
    fn add_dropped_marks_neither_plane() {
        let at = at_bucket(72);
        let stats = Stats::default();
        stats.record_success(at, StdDuration::ZERO, 4);
        stats.add_dropped(4);

        let snap = stats.snapshot_at(clock(at));
        assert_eq!(buckets_set(&snap.activity_fail), Vec::<usize>::new());
        assert_eq!(buckets_set(&snap.activity_ok), vec![72]);
        assert_eq!(snap.total_dropped, 4);
    }

    #[test]
    fn stale_is_derived_on_read_but_never_masks_a_failure() {
        // Never ran and already overdue: stale, and with nothing to report about a
        // check that hasn't happened.
        let never = Stats::default();
        never.set_next_check(1_000, 2_000);
        let snap = never.snapshot_at(clock(3_000));
        assert_eq!(snap.state, ItemState::Stale);
        assert_eq!(snap.last_duration_ms, None);
        assert_eq!(snap.last_record_count, None);

        // Failing outranks stale: an erroring target is the more actionable fact.
        let failing = Stats::default();
        failing.set_next_check(1_000, 2_000);
        failing.record_failure(
            failure(FailureKind::Unreachable, "down"),
            StdDuration::ZERO,
            0,
            false,
        );
        assert_eq!(failing.snapshot_at(clock(3_000)).state, ItemState::Failing);

        // A retry that never happened is stale — that's the scheduler, not the target.
        let retrying = Stats::default();
        retrying.set_next_check(1_000, 2_000);
        retrying.record_failure(
            failure(FailureKind::Timeout, "slow"),
            StdDuration::ZERO,
            1,
            true,
        );
        assert_eq!(retrying.snapshot_at(clock(3_000)).state, ItemState::Stale);
    }

    #[test]
    fn failure_reason_is_truncated_on_write() {
        let long = "e".repeat(MAX_REASON_LEN * 2);
        let f = failure(FailureKind::Malformed, &long);
        assert_eq!(f.reason.len(), MAX_REASON_LEN);
    }

    /// Multi-byte input must not be cut mid-character.
    #[test]
    fn failure_reason_truncation_respects_char_boundaries() {
        let long = "é".repeat(MAX_REASON_LEN * 2);
        let f = failure(FailureKind::Malformed, &long);
        assert_eq!(f.reason.chars().count(), MAX_REASON_LEN);
    }

    fn change(from: ItemState, to: ItemState) -> Option<Transition> {
        Some(Transition { from, to })
    }

    #[test]
    fn record_methods_report_each_state_change_once() {
        let stats = Stats::default();
        assert_eq!(
            stats.record_success(1_000, StdDuration::ZERO, 1),
            change(ItemState::NeverRan, ItemState::Ok)
        );
        assert_eq!(stats.record_success(2_000, StdDuration::ZERO, 1), None);
        assert_eq!(
            stats.record_success(3_000, StdDuration::ZERO, 0),
            change(ItemState::Ok, ItemState::OkEmpty)
        );
        assert_eq!(
            stats.record_failure(
                failure(FailureKind::Timeout, "a"),
                StdDuration::ZERO,
                1,
                true
            ),
            change(ItemState::OkEmpty, ItemState::Retrying)
        );
        assert_eq!(
            stats.record_failure(
                failure(FailureKind::Timeout, "b"),
                StdDuration::ZERO,
                2,
                true
            ),
            None
        );
        assert_eq!(
            stats.record_failure(
                failure(FailureKind::Timeout, "c"),
                StdDuration::ZERO,
                0,
                false
            ),
            change(ItemState::Retrying, ItemState::Failing)
        );
        assert_eq!(
            stats.record_success(4_000, StdDuration::ZERO, 1),
            change(ItemState::Failing, ItemState::Ok)
        );
    }

    #[test]
    fn record_ingest_reports_the_first_match() {
        let stats = Stats::default();
        assert_eq!(
            stats.record_ingest(1_000, 3),
            change(ItemState::NeverRan, ItemState::Ok)
        );
        assert_eq!(stats.record_ingest(2_000, 3), None);
    }

    /// Every event on [`STATE_LOG_TARGET`] emitted while `f` runs, as JSON objects.
    fn state_events(f: impl FnOnce()) -> Vec<serde_json::Value> {
        use std::io;
        use std::sync::Mutex;
        use tracing_subscriber::filter::{LevelFilter, Targets};
        use tracing_subscriber::prelude::*;

        #[derive(Clone, Default)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
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

        let capture = Capture::default();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(capture.clone())
                .with_filter(Targets::default().with_target(STATE_LOG_TARGET, LevelFilter::TRACE)),
        );
        tracing::subscriber::with_default(subscriber, f);
        let bytes = capture.0.lock().unwrap().clone();
        String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn transition_log_is_a_self_describing_event_on_its_own_target() {
        let id = Uuid::now_v7();
        let cause = failure(FailureKind::Auth, "bad community");
        let events = state_events(|| {
            Transition {
                from: ItemState::NeverRan,
                to: ItemState::Ok,
            }
            .log(id, "cpu", None);
            Transition {
                from: ItemState::Ok,
                to: ItemState::Failing,
            }
            .log(id, "cpu", Some(&cause));
        });
        assert_eq!(events.len(), 2);
        for event in &events {
            assert_eq!(event["target"], STATE_LOG_TARGET);
            assert_eq!(event["level"], "INFO");
            assert_eq!(event["fields"]["message"], "item state changed");
            assert_eq!(event["fields"]["item_id"], id.to_string());
            assert_eq!(event["fields"]["item"], "cpu");
        }
        assert_eq!(events[0]["fields"]["from"], "never_ran");
        assert_eq!(events[0]["fields"]["to"], "ok");
        assert!(events[0]["fields"].get("failure_kind").is_none());
        assert!(events[0]["fields"].get("reason").is_none());
        assert_eq!(events[1]["fields"]["from"], "ok");
        assert_eq!(events[1]["fields"]["to"], "failing");
        assert_eq!(events[1]["fields"]["failure_kind"], "auth");
        assert_eq!(events[1]["fields"]["reason"], "bad community");
    }
}
