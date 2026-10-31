// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! `cache::item::Stats` → protobuf conversion for the `GetItemStats` RPC.
//!
//! Lives here rather than in `cache` so the cache stays free of wire types. The
//! two enums carry matching discriminants by construction (see the comments on
//! `cache::item::ItemState` and `FailureKind`), but they're mapped explicitly
//! anyway: a silent `as i32` would let a future variant added on one side ship
//! as a wrong-but-valid value on the other.

use super::comm::ItemStats;
use super::v1;
use crate::cache;
use crate::cache::item::{
    ACTIVITY_PLANE_BYTES, Failure, FailureKind, ItemState, Stats, StatsSnapshot,
};
use anyhow::{Context, Result};
use bytes::Bytes;
use chrono::{DateTime, FixedOffset};
use futures::{Stream, StreamExt};
use prost::Message;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use tonic::Status;
use tracing::trace;
use uuid::Uuid;

impl From<ItemState> for v1::ItemState {
    fn from(s: ItemState) -> Self {
        match s {
            ItemState::Unknown => Self::Unspecified,
            ItemState::NeverRan => Self::NeverRan,
            ItemState::Ok => Self::Ok,
            ItemState::OkEmpty => Self::OkEmpty,
            ItemState::Retrying => Self::Retrying,
            ItemState::Failing => Self::Failing,
            ItemState::Stale => Self::Stale,
        }
    }
}

impl From<v1::ItemState> for ItemState {
    fn from(s: v1::ItemState) -> Self {
        match s {
            v1::ItemState::Unspecified => Self::Unknown,
            v1::ItemState::NeverRan => Self::NeverRan,
            v1::ItemState::Ok => Self::Ok,
            v1::ItemState::OkEmpty => Self::OkEmpty,
            v1::ItemState::Retrying => Self::Retrying,
            v1::ItemState::Failing => Self::Failing,
            v1::ItemState::Stale => Self::Stale,
        }
    }
}

impl From<FailureKind> for v1::FailureKind {
    fn from(k: FailureKind) -> Self {
        match k {
            FailureKind::Timeout => Self::Timeout,
            FailureKind::Unreachable => Self::Unreachable,
            FailureKind::Auth => Self::Auth,
            FailureKind::NotSupported => Self::NotSupported,
            FailureKind::TargetError => Self::TargetError,
            FailureKind::Malformed => Self::Malformed,
            FailureKind::Config => Self::Config,
            FailureKind::Dropped => Self::Dropped,
            FailureKind::Cluster => Self::Cluster,
            FailureKind::Internal => Self::Internal,
        }
    }
}

impl From<&Failure> for v1::ItemFailure {
    fn from(f: &Failure) -> Self {
        Self {
            at_unix_ms: f.at.timestamp_millis(),
            kind: v1::FailureKind::from(f.kind) as i32,
            reason: f.reason.to_string(),
            detail_json: encode_detail(f),
        }
    }
}

/// JSON-encode a failure's structured detail, same convention as
/// `CaptureMessage`.
///
/// Detail is diagnostic garnish, so an encoding error drops it rather than
/// failing the surrounding RPC — losing the OID list on one failure is a far
/// better outcome than a stats page that won't load. In practice this can only
/// fail on a non-string map key or a non-finite float reaching `Value`.
fn encode_detail(f: &Failure) -> Bytes {
    let Some(detail) = f.detail.as_deref() else {
        return Bytes::new();
    };
    match serde_json::to_vec(detail) {
        Ok(bytes) => bytes.into(),
        Err(e) => {
            trace!(kind = %f.kind, error = %e, "failed to json-encode failure detail");
            Bytes::new()
        }
    }
}

/// Build the wire form of one item's stats.
///
/// `node_id` is the node answering the request, not the item's configured
/// owner: for listener items several nodes legitimately report the same item and
/// the caller needs to know who said what in order to merge them.
///
/// `include_recent_failures` is the inverse of the request's
/// `exclude_recent_failures` — the ring is most of the payload, and a table view
/// only needs `last_failure`.
///
/// The activity planes pass through untouched: a snapshot already carries them in
/// the wire layout, masked, with an all-zero plane already collapsed to empty — the
/// same "absent is empty, not a placeholder" convention as `detail_json`. Cloning a
/// `Bytes` is a refcount bump, so a large stats read copies no plane bytes at all.
pub fn to_proto(
    item_id: Uuid,
    node_id: Uuid,
    snap: &StatsSnapshot,
    include_recent_failures: bool,
) -> v1::ItemStats {
    v1::ItemStats {
        item_id: item_id.as_bytes().to_vec().into(),
        node_id: node_id.as_bytes().to_vec().into(),

        state: v1::ItemState::from(snap.state) as i32,

        total_checks: snap.total_checks,
        total_failures: snap.total_failures,
        total_records: snap.total_records,
        total_dropped: snap.total_dropped,
        consecutive_failures: snap.consecutive_failures,
        retries: snap.retries as u32,

        last_attempt_unix_ms: snap.last_attempt.map(|t| t.timestamp_millis()),
        last_success_unix_ms: snap.last_success.map(|t| t.timestamp_millis()),
        last_nonempty_unix_ms: snap.last_nonempty.map(|t| t.timestamp_millis()),
        next_check_unix_ms: snap.next_check.map(|t| t.timestamp_millis()),

        last_duration_ms: snap.last_duration_ms,
        last_lag_ms: snap.last_lag_ms,
        last_record_count: snap.last_record_count,

        activity_ok: snap.activity_ok.clone(),
        activity_fail: snap.activity_fail.clone(),
        activity_window_end_unix_ms: snap.activity_window_end.timestamp_millis(),

        last_failure: snap.last_failure.as_deref().map(v1::ItemFailure::from),
        recent_failures: match (include_recent_failures, &snap.recent_failures) {
            // Ring is stored newest-first, which is also the wire order.
            (true, Some(ring)) => ring.iter().map(v1::ItemFailure::from).collect(),
            _ => Vec::new(),
        },
    }
}

// ---------------------------------------------------------------------------
// Wire → snapshot
//
// The receiving half, for the API node fanning `GetItemStats` across the cluster.
//
// It is deliberately *not* the mirror image of `to_proto`. That direction builds a
// message from atomics it read moments earlier and can assume every value is
// well-formed; this one parses whatever a peer sent. So the two halves draw the
// fallibility line in different places: **identity is validated, diagnostics
// degrade.** A snapshot attributed to the wrong item is worse than no snapshot, so a
// malformed id fails the item. A sparkline that cannot be decoded is just a
// sparkline, so it comes back empty and the counters and failures still arrive.
// ---------------------------------------------------------------------------

/// One peer's answer for one item.
///
/// The ids ride along because a fan-out interleaves both: response chunks arrive in
/// no particular order, and a listener item legitimately appears in several nodes'
/// answers, which is the whole reason `node_id` is on the wire. Named fields rather
/// than a `(Uuid, Uuid, StatsSnapshot)` tuple — two same-typed ids side by side are
/// exactly the pair that gets silently swapped at a call site.
#[derive(Debug, Clone)]
pub struct RemoteStats {
    pub item_id: Uuid,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "decoded from the wire, only tests read it yet")
    )]
    pub node_id: Uuid,
    pub stats: StatsSnapshot,
}

/// A wire timestamp as a local instant, or `None` if it names no representable one.
///
/// The zone is the **receiving** node's, because the wire carries an instant and
/// nothing else. That choice matters for the merge: a fan-out interleaves peers'
/// answers with this node's own [`Stats::snapshot`], and rendering half the rows at a
/// different offset than the other half would look like a bug.
///
/// `None` needs a timestamp beyond ±262,000 years, which no clock produces — but
/// a panic is fatal (`common::panic`), so an `unwrap` here would take the process
/// down over one peer's bad field; it is an `Option` rather than an assumption.
fn to_local(unix_ms: i64) -> Option<DateTime<FixedOffset>> {
    to_local_in(unix_ms, &crate::common::DEFAULT_TZ)
}

/// [`to_local`] against an explicit zone.
///
/// Split out to be testable: `DEFAULT_TZ` is read from the environment once per
/// process, so under an unset or UTC `$TZ` — a CI runner, say — asserting against it
/// proves nothing about whether a zone is applied at all.
///
/// Resolved per instant rather than stamped from "now", which is what makes a
/// timestamp from the other side of a DST change carry the offset that was in force
/// *then*. `with_timezone` takes the instant → local direction, which is always
/// unambiguous; the ambiguity that needs a `MappedLocalTime` is the other way round.
fn to_local_in(unix_ms: i64, tz: &chrono_tz::Tz) -> Option<DateTime<FixedOffset>> {
    DateTime::from_timestamp_millis(unix_ms).map(|t| t.with_timezone(tz).fixed_offset())
}

/// JSON-decode a failure's structured detail. The inverse of [`encode_detail`], with
/// the same policy: detail is garnish, so anything unreadable is dropped rather than
/// failing the failure that carries it.
fn decode_detail(detail_json: &Bytes) -> Option<serde_json::Value> {
    if detail_json.is_empty() {
        return None;
    }
    match serde_json::from_slice(detail_json) {
        Ok(v) => Some(v),
        Err(e) => {
            trace!(error = %e, "failed to json-decode failure detail");
            None
        }
    }
}

/// An activity plane as the [`StatsSnapshot`] contract requires it: empty, or exactly
/// [`ACTIVITY_PLANE_BYTES`].
///
/// Any other length is dropped to empty rather than passed through. A short or long
/// plane can only mean a peer built against a different bucket count, and there is no
/// safe reading of it — decoding 288 positions out of 40 bytes would put every bucket
/// at the wrong time of day, which is worse than showing nothing, because it looks
/// like data. Empty renders as "no result", which is the truth. It does not fail the
/// item: one unreadable sparkline is no reason to withhold its counters.
fn plane_from_wire(plane: Bytes, which: &str) -> Bytes {
    if plane.is_empty() || plane.len() == ACTIVITY_PLANE_BYTES {
        return plane;
    }
    trace!(
        len = plane.len(),
        expected = ACTIVITY_PLANE_BYTES,
        plane = which,
        "discarding activity plane of unexpected length"
    );
    Bytes::new()
}

impl From<v1::FailureKind> for FailureKind {
    /// `Unspecified` — and, via [`RemoteStats`], any value this build doesn't know —
    /// becomes [`FailureKind::Internal`].
    ///
    /// Neither an error nor a new `Unknown` variant. A peer naming a kind we've never
    /// heard of is a version skew inside patrolli itself, and "a patrolli bug, not the
    /// user's problem" is precisely what `Internal` already means. Erroring instead
    /// would discard a real, timestamped, reasoned failure over its label, and adding
    /// an `Unknown` variant would oblige forty-odd construction sites and every
    /// retryability decision to consider a case only this function can produce.
    fn from(k: v1::FailureKind) -> Self {
        match k {
            v1::FailureKind::Timeout => Self::Timeout,
            v1::FailureKind::Unreachable => Self::Unreachable,
            v1::FailureKind::Auth => Self::Auth,
            v1::FailureKind::NotSupported => Self::NotSupported,
            v1::FailureKind::TargetError => Self::TargetError,
            v1::FailureKind::Malformed => Self::Malformed,
            v1::FailureKind::Config => Self::Config,
            v1::FailureKind::Dropped => Self::Dropped,
            v1::FailureKind::Cluster => Self::Cluster,
            v1::FailureKind::Internal | v1::FailureKind::Unspecified => Self::Internal,
        }
    }
}

impl TryFrom<v1::ItemFailure> for Failure {
    type Error = anyhow::Error;

    /// Fails only on a timestamp that names no real instant. A failure is a point on
    /// a timeline; one that can't be placed on it has nothing left to say, whereas an
    /// unknown kind or unreadable detail still leaves a time and a reason.
    ///
    /// Built through [`Failure::new`], so `reason` is re-truncated here. The producer
    /// truncated it too, but a peer is input and the cap belongs at the boundary.
    fn try_from(f: v1::ItemFailure) -> Result<Self> {
        let at = to_local(f.at_unix_ms)
            .with_context(|| format!("failure timestamp {} out of range", f.at_unix_ms))?;
        let kind = v1::FailureKind::try_from(f.kind)
            .map(FailureKind::from)
            .unwrap_or(FailureKind::Internal);
        let detail = decode_detail(&f.detail_json);
        Ok(Failure::new(at, kind, &f.reason, detail))
    }
}

/// Convert a wire failure, dropping it if it can't be placed in time.
fn failure_from_wire(f: v1::ItemFailure) -> Option<Failure> {
    match Failure::try_from(f) {
        Ok(f) => Some(f),
        Err(e) => {
            trace!(error = %e, "dropping unconvertible failure");
            None
        }
    }
}

impl TryFrom<v1::ItemStats> for RemoteStats {
    type Error = anyhow::Error;

    /// Parse one item's stats off the wire.
    ///
    /// Errors only on a malformed `item_id` or `node_id`. Everything else has a
    /// truthful degraded reading and takes it: an unrecognised state becomes
    /// [`ItemState::Unknown`] — which is what that variant is for, "a state we
    /// couldn't interpret" — an unrecognised failure kind becomes
    /// [`FailureKind::Internal`], a `retries` above `u8::MAX` clamps rather than
    /// wrapping to a small number, and a plane or timestamp that doesn't parse comes
    /// back absent.
    fn try_from(msg: v1::ItemStats) -> Result<Self> {
        let item_id = Uuid::from_slice(&msg.item_id).context("item_id")?;
        let node_id = Uuid::from_slice(&msg.node_id).context("node_id")?;

        let state = v1::ItemState::try_from(msg.state)
            .map(ItemState::from)
            .unwrap_or(ItemState::Unknown);

        // The planes are positions *relative to* the window end, so without a usable
        // end they aren't merely odd, they're unreadable — every bucket would be
        // pinned to the wrong five minutes. Drop all three together.
        let (activity_ok, activity_fail, activity_window_end) =
            match to_local(msg.activity_window_end_unix_ms) {
                Some(end) => (
                    plane_from_wire(msg.activity_ok, "ok"),
                    plane_from_wire(msg.activity_fail, "fail"),
                    end,
                ),
                None => {
                    trace!(
                        end = msg.activity_window_end_unix_ms,
                        "discarding activity window with an out-of-range end"
                    );
                    (Bytes::new(), Bytes::new(), DateTime::default())
                }
            };

        // An empty ring is `None`, not `Some(vec![])`. This is the one place the
        // conversion genuinely loses information: `to_proto` sends an empty ring both
        // for an item that has never failed and for a response that excluded it, so
        // the two are indistinguishable here. A `last_failure` alongside an empty ring
        // is the tell.
        let recent: Vec<Failure> = msg
            .recent_failures
            .into_iter()
            .filter_map(failure_from_wire)
            .collect();

        Ok(Self {
            item_id,
            node_id,
            stats: StatsSnapshot {
                state,

                total_checks: msg.total_checks,
                total_failures: msg.total_failures,
                total_records: msg.total_records,
                total_dropped: msg.total_dropped,
                consecutive_failures: msg.consecutive_failures,
                retries: msg.retries.min(u32::from(u8::MAX)) as u8,

                last_attempt: msg.last_attempt_unix_ms.and_then(to_local),
                last_success: msg.last_success_unix_ms.and_then(to_local),
                last_nonempty: msg.last_nonempty_unix_ms.and_then(to_local),
                next_check: msg.next_check_unix_ms.and_then(to_local),

                last_duration_ms: msg.last_duration_ms,
                last_lag_ms: msg.last_lag_ms,
                last_record_count: msg.last_record_count,

                activity_ok,
                activity_fail,
                activity_window_end,

                last_failure: msg.last_failure.and_then(failure_from_wire).map(Arc::new),
                recent_failures: (!recent.is_empty()).then(|| Arc::new(recent)),
            },
        })
    }
}

/// How much encoded payload a chunk aims to carry before it ships.
///
/// Sized in bytes rather than item count because a failure's `detail_json` is
/// unbounded, so "256 items" is not a size.
///
/// The ceiling that matters is not `MAX_GRPC_MESSAGE_SIZE`: the server raises its
/// own decode limit to that, but the inter-node client is built bare and so keeps
/// tonic's 4 MiB default. A chunk above 4 MiB would therefore be undecodable at the
/// far end. 1 MiB leaves room for the envelope and for one item that overshoots.
const CHUNK_TARGET_BYTES: usize = 1024 * 1024;

/// Which item states a request asked for.
pub(super) struct StateFilter(Option<Vec<ItemState>>);

impl StateFilter {
    /// The request's `states`, where empty means no filter at all.
    ///
    /// A wire value this build doesn't recognise contributes no variant, which is
    /// exactly right — no local item can be in a state we've never heard of. But it
    /// makes the empty-versus-absent distinction load-bearing: `[]` is `None` and
    /// matches everything, while `[99]` is `Some(vec![])` and matches nothing.
    /// Collapsing the two would turn "show me what's broken" into "show me the
    /// entire fleet".
    pub(super) fn new(states: &[i32]) -> Self {
        if states.is_empty() {
            return Self(None);
        }
        Self(Some(
            states
                .iter()
                .filter_map(|s| v1::ItemState::try_from(*s).ok())
                .map(ItemState::from)
                .collect(),
        ))
    }

    pub(super) fn matches(&self, state: ItemState) -> bool {
        match &self.0 {
            None => true,
            Some(wanted) => wanted.contains(&state),
        }
    }
}

/// Resolve requested ids against the item cache: what this node holds, and which of
/// the requested ids it doesn't. Empty `ids` resolves to nothing, without touching
/// the lock — see `ItemStatsRequest.item_ids`: there is no "every item" form.
///
/// Takes and releases the read lock, and does nothing else under it — no snapshot,
/// no serialisation, per the contract on [`cache::item::Item::stats`]. Shared by the
/// serving path and by this node's own contribution to a fan-out, so the lock
/// discipline is stated once.
pub(super) fn gather(
    cache: &parking_lot::RwLock<cache::item::Item>,
    ids: &[Uuid],
) -> (Vec<(Uuid, Arc<Stats>)>, Vec<Uuid>) {
    if ids.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let cache = cache.read();
    let mut found = Vec::with_capacity(ids.len());
    let mut unknown = Vec::new();
    for id in ids {
        // Absent from the cache means "not owned here". An item that is here but
        // fails the `states` filter is a different answer and stays out of this list.
        match cache.stats(id) {
            Some(stats) => found.push((*id, stats)),
            None => unknown.push(*id),
        }
    }
    (found, unknown)
}

/// Parse the `item_ids` of a request.
///
/// Fails the whole call rather than skipping the offender. A malformed uuid can only
/// come from a patrolli peer, so it is a bug in patrolli, and reporting it as
/// `unknown_item_ids` — "no stats for that" — would bury it in a legitimate answer.
/// The same line [`RemoteStats`] draws in the other direction: identity is validated.
///
/// `invalid_argument` rather than the `internal` that `processor_capture` returns for
/// its own id: four bytes where a uuid belongs is the caller's mistake, and the
/// status code is how it gets told.
pub(super) fn parse_ids(ids: &[Bytes]) -> Result<Vec<Uuid>, Status> {
    ids.iter()
        .map(|id| {
            Uuid::from_slice(id).map_err(|e| Status::invalid_argument(format!("item_id: {e}")))
        })
        .collect()
}

/// One node's answer to a `GetItemStats` request, resolved and ready to serialise.
///
/// The cache lookup happens in the RPC method, not here, because it holds a lock
/// that must be released before any of this runs.
pub(super) struct Sweep {
    /// The node answering. Stamped on every chunk and on every `ItemStats` within
    /// it, because a listener item legitimately accumulates stats on several nodes.
    pub(super) node_id: Uuid,
    /// Read once for the whole sweep so every item is judged against the same
    /// instant. Not merely cosmetic: `ItemState::Stale` is derived from it.
    pub(super) now: DateTime<FixedOffset>,
    pub(super) stats_since_unix_ms: i64,
    /// The inverse of the request's `exclude_recent_failures`.
    pub(super) include_recent_failures: bool,
    pub(super) states: StateFilter,
    /// Ids paired with counters, already out from under the cache lock.
    pub(super) items: Vec<(Uuid, Arc<Stats>)>,
    /// Requested ids this node holds no entry for. Rides on the final chunk.
    pub(super) unknown_item_ids: Vec<Uuid>,
}

impl Sweep {
    /// The sweep as chunked responses, built lazily.
    ///
    /// Three properties a caller depends on:
    ///
    /// - **Every chunk carries `node_id` and `stats_since_unix_ms`.** Reading the
    ///   rest must not require having kept the first one.
    /// - **Exactly one chunk always arrives**, even when nothing matched. It is how
    ///   `stats_since_unix_ms` and `unknown_item_ids` reach a caller whose query came
    ///   up empty, and it keeps "asked, and the answer was nothing" distinct from
    ///   "never answered".
    /// - **Only the last chunk names `unknown_item_ids`**, so the caller can tell
    ///   "not owned here" from "owned but silent". Which chunk is last isn't known
    ///   until the items run out, hence the one-chunk lookahead below.
    ///
    /// Lazy rather than collected: memory stays proportional to a chunk instead of
    /// the fleet, every `yield` is a point where a million-item sweep hands back the
    /// executor thread, and tonic's send window makes a slow caller slow production
    /// rather than buffer it.
    pub(super) fn into_stream(self) -> impl Stream<Item = v1::ItemStatsResponse> {
        let Self {
            node_id,
            now,
            stats_since_unix_ms,
            include_recent_failures,
            states,
            items,
            unknown_item_ids,
        } = self;

        async_stream::stream! {
            let node_bytes: Bytes = node_id.as_bytes().to_vec().into();
            let chunk = |stats: Vec<v1::ItemStats>| v1::ItemStatsResponse {
                stats,
                node_id: node_bytes.clone(),
                stats_since_unix_ms,
                unknown_item_ids: Vec::new(),
            };

            let mut pending: Option<v1::ItemStatsResponse> = None;
            let mut batch: Vec<v1::ItemStats> = Vec::new();
            let mut batch_bytes = 0usize;

            for (item_id, stats) in items {
                let snap = stats.snapshot_at(now);
                // Filter on the snapshot's state, never the stored byte: `Stale` is
                // derived on read, so an item stored `Ok` whose deadline has passed
                // answers to a `Stale` filter and not to an `Ok` one.
                if !states.matches(snap.state) {
                    continue;
                }
                let msg = to_proto(item_id, node_id, &snap, include_recent_failures);
                let len = msg.encoded_len();
                // Gated on a non-empty batch so an item larger than the entire
                // budget ships alone instead of being deferred forever.
                if !batch.is_empty() && batch_bytes + len > CHUNK_TARGET_BYTES {
                    if let Some(ready) = pending.replace(chunk(std::mem::take(&mut batch))) {
                        yield ready;
                    }
                    batch_bytes = 0;
                }
                batch_bytes += len;
                batch.push(msg);
            }
            if !batch.is_empty() && let Some(ready) = pending.replace(chunk(batch)) {
                yield ready;
            }

            let mut last = pending.unwrap_or_else(|| chunk(Vec::new()));
            last.unknown_item_ids = unknown_item_ids
                .iter()
                .map(|id| id.as_bytes().to_vec().into())
                .collect();
            yield last;
        }
    }
}

/// This node's own stats, in the merged form, read straight from the cache.
///
/// Deliberately not `Sweep` + [`RemoteStats`]. That pair exists to survive a wire
/// crossing; there is no wire here. Going through it would pack the activity planes
/// into bytes, reduce every timestamp to millis, and then immediately undo both — and
/// it would put this node's own numbers through a lossy `retries` clamp for nothing.
pub(super) fn local_stats(
    node_id: Uuid,
    item_cache: &parking_lot::RwLock<cache::item::Item>,
    req: &v1::ItemStatsRequest,
) -> Result<ItemStats, Status> {
    let ids = parse_ids(&req.item_ids)?;
    let (items, _unknown) = gather(item_cache, &ids);
    Ok(merge_local(
        node_id,
        // One clock for all of them, matching the serving path: `Stale` is derived
        // from it, so reading it per item would judge items by different deadlines.
        crate::common::now().fixed_offset(),
        crate::common::get_start_datetime().fixed_offset(),
        items,
        &StateFilter::new(&req.states),
    ))
}

/// Snapshot and filter already-gathered items into a per-node answer.
///
/// Split from [`local_stats`] so the part with rules in it can be tested against a
/// hand-built `Vec` — the same shape `Sweep` takes, which is what lets one test feed
/// both paths identical input and compare the results.
pub(super) fn merge_local(
    node_id: Uuid,
    now: DateTime<FixedOffset>,
    since: DateTime<FixedOffset>,
    items: Vec<(Uuid, Arc<Stats>)>,
    states: &StateFilter,
) -> ItemStats {
    ItemStats {
        stats: items
            .into_iter()
            .map(|(id, stats)| (id, stats.snapshot_at(now)))
            .filter(|(_, snap)| states.matches(snap.state))
            .collect(),
        node_id,
        since: Some(since),
        error: None,
    }
}

/// Fold one peer's response chunks into a single answer.
///
/// Never fails. A stream that breaks partway sets `error` and keeps everything that
/// already arrived, because half a node's stats plus the reason for the other half is
/// strictly more than nothing — and the caller can already tell, because `error` is
/// set. Same for a single item that won't convert: it is dropped and the other
/// thousand still land, which is the line [`RemoteStats`] already draws.
///
/// A stream that ends without one chunk *is* an error. The server owes at least one
/// even when nothing matched, so zero means the peer isn't serving this RPC properly
/// and reporting an empty success would read as "this node has no items".
pub(super) async fn collect_node<S>(node_id: Uuid, chunks: S) -> ItemStats
where
    S: Stream<Item = Result<v1::ItemStatsResponse, Status>>,
{
    let mut out = ItemStats {
        stats: FxHashMap::default(),
        node_id,
        since: None,
        error: None,
    };
    let mut seen_chunk = false;

    let mut chunks = std::pin::pin!(chunks);
    while let Some(chunk) = chunks.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                out.error = Some(e.to_string());
                return out;
            }
        };
        // Both fields ride on every chunk, so the first one to arrive settles them
        // and the rest are redundant. Taken from the first rather than the last so a
        // stream that breaks midway still reports a `since`.
        if !seen_chunk {
            seen_chunk = true;
            out.since = to_local(chunk.stats_since_unix_ms);
        }
        for msg in chunk.stats {
            match RemoteStats::try_from(msg) {
                Ok(remote) => {
                    out.stats.insert(remote.item_id, remote.stats);
                }
                Err(e) => trace!(%node_id, error = %e, "dropping unconvertible item stats"),
            }
        }
    }

    if !seen_chunk {
        out.error = Some("peer closed the stream without sending a chunk".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::item::Stats;
    use serde_json::json;

    fn failure(kind: FailureKind) -> Failure {
        Failure {
            at: crate::common::now().fixed_offset(),
            kind,
            reason: "request timeout".into(),
            detail: Some(Box::new(json!({"oid": "1.3.6.1.2.1.1.3.0"}))),
        }
    }

    /// Unix millis as the instant `snapshot_at` reads at. The offset only decides how
    /// the snapshot renders its timestamps; `to_proto` puts millis back on the wire,
    /// so these tests assert against the millis they wrote.
    fn clock(ms: i64) -> DateTime<FixedOffset> {
        DateTime::from_timestamp_millis(ms)
            .expect("test timestamp in range")
            .fixed_offset()
    }

    fn cached_item() -> crate::config::item::Item {
        use crate::config::item::{Config, Dummy, DummyPayload, Item, Schedule};
        Item::new(
            Uuid::now_v7(),
            "test".into(),
            None,
            Uuid::now_v7(),
            Schedule::Never,
            None,
            std::time::Duration::from_secs(1),
            vec![],
            Config::Dummy(Dummy {
                payload: DummyPayload::default(),
                count: 1,
            }),
        )
    }

    /// Empty ids are "nothing", not "everything": a node can hold millions of
    /// items, and a whole-map sweep must not be one missing argument away.
    #[test]
    fn gather_with_no_ids_returns_nothing() {
        let cache = parking_lot::RwLock::new(cache::item::Item::new());
        let item = cached_item();
        let id = item.id;
        cache.write().insert(item);

        let (found, unknown) = gather(&cache, &[]);
        assert!(found.is_empty());
        assert!(unknown.is_empty());

        let missing = Uuid::now_v7();
        let (found, unknown) = gather(&cache, &[id, missing]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, id);
        assert_eq!(unknown, vec![missing]);
    }

    /// A default `Stats` must convert without inventing timestamps: an item the
    /// scheduler hasn't reached yet should read as "nothing known", not as an
    /// item that last succeeded at the unix epoch.
    #[test]
    fn empty_stats_reports_absent_rather_than_zero() {
        let snap = Stats::default().snapshot();
        let out = to_proto(Uuid::now_v7(), Uuid::now_v7(), &snap, true);

        assert_eq!(out.state, v1::ItemState::NeverRan as i32);
        assert_eq!(out.last_success_unix_ms, None);
        assert_eq!(out.last_attempt_unix_ms, None);
        assert_eq!(out.last_record_count, None);
        assert!(out.activity_ok.is_empty());
        assert!(out.activity_fail.is_empty());
        assert!(out.last_failure.is_none());
        assert!(out.recent_failures.is_empty());
    }

    #[test]
    fn failure_detail_round_trips_as_json_bytes() {
        let f = failure(FailureKind::Timeout);
        let out = v1::ItemFailure::from(&f);

        assert_eq!(out.kind, v1::FailureKind::Timeout as i32);
        assert_eq!(out.reason, "request timeout");
        assert_eq!(out.at_unix_ms, f.at.timestamp_millis());
        let decoded: serde_json::Value = serde_json::from_slice(&out.detail_json).unwrap();
        assert_eq!(decoded, json!({"oid": "1.3.6.1.2.1.1.3.0"}));
    }

    #[test]
    fn absent_detail_encodes_as_empty_not_null() {
        let mut f = failure(FailureKind::Config);
        f.detail = None;
        // `null` would be 4 bytes that every consumer then has to special-case.
        assert!(v1::ItemFailure::from(&f).detail_json.is_empty());
    }

    #[test]
    fn exclude_recent_failures_keeps_last_failure() {
        let snap = StatsSnapshot {
            last_failure: Some(Arc::new(failure(FailureKind::Auth))),
            recent_failures: Some(Arc::new(vec![
                failure(FailureKind::Auth),
                failure(FailureKind::Timeout),
            ])),
            ..Default::default()
        };

        let full = to_proto(Uuid::now_v7(), Uuid::now_v7(), &snap, true);
        assert_eq!(full.recent_failures.len(), 2);
        assert!(full.last_failure.is_some());

        let slim = to_proto(Uuid::now_v7(), Uuid::now_v7(), &snap, false);
        assert!(slim.recent_failures.is_empty());
        assert!(slim.last_failure.is_some());
    }

    /// End-to-end over the real write API: what the monitor records is what the
    /// peer receives.
    #[test]
    fn a_populated_stats_round_trips_through_the_wire_form() {
        use std::time::Duration;

        let stats = Stats::default();
        stats.record_attempt(1_000, Duration::from_millis(40));
        stats.record_success(1_000, Duration::from_millis(12), 7);
        stats.record_attempt(2_000, Duration::ZERO);
        stats.record_failure(
            failure(FailureKind::Timeout),
            Duration::from_millis(30),
            1,
            true,
        );
        stats.add_dropped(2);
        stats.set_next_check(3_000, 9_000);

        let snap = stats.snapshot_at(clock(2_000));
        let out = to_proto(Uuid::now_v7(), Uuid::now_v7(), &snap, true);

        assert_eq!(out.state, v1::ItemState::Retrying as i32);
        assert_eq!(out.total_checks, 2);
        assert_eq!(out.total_failures, 1);
        assert_eq!(out.total_records, 7);
        assert_eq!(out.total_dropped, 2);
        assert_eq!(out.consecutive_failures, 1);
        assert_eq!(out.retries, 1);
        assert_eq!(out.last_success_unix_ms, Some(1_000));
        assert_eq!(out.last_nonempty_unix_ms, Some(1_000));
        assert_eq!(out.last_attempt_unix_ms, Some(2_000));
        assert_eq!(out.next_check_unix_ms, Some(3_000));
        assert_eq!(out.last_duration_ms, Some(30));

        let last = out.last_failure.expect("failure crossed the wire");
        assert_eq!(last.kind, v1::FailureKind::Timeout as i32);
        assert_eq!(out.recent_failures.len(), 1);
    }

    // --- windowed activity planes ---

    const BUCKET_MS: i64 = 5 * 60 * 1000;

    /// Unix millis for bucket `n` of a lap far from the epoch, so nothing
    /// accidentally passes on a zero. No timezone involved — buckets are absolute
    /// 5-minute ticks modulo 288.
    fn at_bucket(n: i64) -> i64 {
        288 * BUCKET_MS * 100_000 + n * BUCKET_MS
    }

    /// The window crossing the wire, end to end over the real write API.
    ///
    /// The planes are already packed by the time `to_proto` sees them, so what this
    /// guards is the mapping: right field, right plane, no reordering.
    #[test]
    fn the_activity_window_round_trips_through_the_wire_form() {
        use std::time::Duration;

        let stats = Stats::default();
        // Bucket 72 succeeded, bucket 73 failed — adjacent, one per plane.
        stats.record_success(at_bucket(72), Duration::from_millis(5), 3);
        stats.record_failure(
            Failure::new(
                clock(at_bucket(73)),
                FailureKind::Timeout,
                "timed out",
                None,
            ),
            Duration::from_millis(30),
            0,
            false,
        );

        let snap = stats.snapshot_at(clock(at_bucket(73)));
        let out = to_proto(Uuid::now_v7(), Uuid::now_v7(), &snap, true);

        // bucket 72 → byte 9 bit 0; bucket 73 → byte 9 bit 1.
        assert_eq!(out.activity_ok[9], 0b0000_0001);
        assert_eq!(out.activity_fail[9], 0b0000_0010);
        // Neither plane leaks into the other's bucket.
        assert_eq!(out.activity_ok[9] & out.activity_fail[9], 0);
        assert_eq!(out.activity_window_end_unix_ms, at_bucket(73));
    }

    /// The two enums are mapped by hand; this is the guard that keeps them
    /// aligned when a variant is added to either side.
    #[test]
    fn enum_discriminants_match_the_proto() {
        for (rust, proto) in [
            (FailureKind::Timeout, v1::FailureKind::Timeout),
            (FailureKind::Unreachable, v1::FailureKind::Unreachable),
            (FailureKind::Auth, v1::FailureKind::Auth),
            (FailureKind::NotSupported, v1::FailureKind::NotSupported),
            (FailureKind::TargetError, v1::FailureKind::TargetError),
            (FailureKind::Malformed, v1::FailureKind::Malformed),
            (FailureKind::Config, v1::FailureKind::Config),
            (FailureKind::Dropped, v1::FailureKind::Dropped),
            (FailureKind::Cluster, v1::FailureKind::Cluster),
            (FailureKind::Internal, v1::FailureKind::Internal),
        ] {
            assert_eq!(rust as i32, proto as i32, "{rust} drifted from the proto");
            assert_eq!(v1::FailureKind::from(rust), proto);
            assert_eq!(FailureKind::from(proto), rust);
        }
        // The one kind that isn't a bijection, and why: see `From<v1::FailureKind>`.
        assert_eq!(
            FailureKind::from(v1::FailureKind::Unspecified),
            FailureKind::Internal
        );

        for (rust, proto) in [
            (ItemState::Unknown, v1::ItemState::Unspecified),
            (ItemState::NeverRan, v1::ItemState::NeverRan),
            (ItemState::Ok, v1::ItemState::Ok),
            (ItemState::OkEmpty, v1::ItemState::OkEmpty),
            (ItemState::Retrying, v1::ItemState::Retrying),
            (ItemState::Failing, v1::ItemState::Failing),
            (ItemState::Stale, v1::ItemState::Stale),
        ] {
            assert_eq!(rust as i32, proto as i32, "{rust} drifted from the proto");
            assert_eq!(v1::ItemState::from(rust), proto);
            assert_eq!(ItemState::from(proto), rust);
        }
    }

    // --- wire → snapshot ---

    /// A well-formed message with valid ids, for the degradation tests to spoil one
    /// field at a time. Built by `to_proto`, so a field that moves in the outbound
    /// mapping can't leave these testing a shape nothing produces.
    fn wire() -> v1::ItemStats {
        to_proto(
            Uuid::now_v7(),
            Uuid::now_v7(),
            &Stats::default().snapshot(),
            true,
        )
    }

    /// The load-bearing test for both directions at once: what one node records is
    /// what another node reads back.
    ///
    /// Asserts the whole snapshot rather than a list of fields, so a field added to
    /// `StatsSnapshot` and wired into only one direction fails here without anyone
    /// remembering to extend the test.
    #[test]
    fn a_snapshot_survives_the_full_round_trip() {
        use std::time::Duration;

        let stats = Stats::default();
        stats.record_attempt(at_bucket(70), Duration::from_millis(40));
        stats.record_success(at_bucket(70), Duration::from_millis(12), 7);
        stats.record_attempt(at_bucket(72), Duration::ZERO);
        stats.record_failure(
            Failure::new(
                clock(at_bucket(72)),
                FailureKind::Timeout,
                "timed out",
                Some(json!({"oid": "1.3.6.1.2.1.1.3.0"})),
            ),
            Duration::from_millis(30),
            1,
            true,
        );
        stats.add_dropped(2);
        stats.set_next_check(at_bucket(73), at_bucket(80));

        let item_id = Uuid::now_v7();
        let node_id = Uuid::now_v7();
        let snap = stats.snapshot_at(clock(at_bucket(72)));

        let back = RemoteStats::try_from(to_proto(item_id, node_id, &snap, true))
            .expect("a message we built ourselves must parse");

        assert_eq!(back.item_id, item_id);
        assert_eq!(back.node_id, node_id);
        // Timestamps compare by instant, which is the right notion: the wire carries
        // no offset, so the zone each side renders in is not part of the value.
        assert_eq!(back.stats, snap);
    }

    /// Identity is the one thing that can't degrade: stats filed against the wrong
    /// item are worse than no stats.
    #[test]
    fn a_malformed_id_fails_the_item() {
        let mut short = wire();
        short.item_id = Bytes::from_static(&[1, 2, 3, 4]);
        let err = RemoteStats::try_from(short).expect_err("a 4-byte item_id is refused");
        assert!(err.to_string().contains("item_id"), "{err}");

        let mut empty = wire();
        empty.node_id = Bytes::new();
        let err = RemoteStats::try_from(empty).expect_err("an absent node_id is refused");
        assert!(err.to_string().contains("node_id"), "{err}");
    }

    /// A peer on a newer build can name a state or kind this one has never heard of.
    /// That must not cost us the rest of the message.
    #[test]
    fn unknown_enum_values_degrade_rather_than_fail() {
        let mut msg = wire();
        msg.state = 99;
        msg.total_checks = 41;
        msg.last_failure = Some(v1::ItemFailure {
            at_unix_ms: at_bucket(10),
            kind: 99,
            reason: "from the future".into(),
            detail_json: Bytes::new(),
        });

        let back = RemoteStats::try_from(msg).expect("unknown enums are not fatal");
        assert_eq!(back.stats.state, ItemState::Unknown);
        let f = back.stats.last_failure.expect("the failure still arrives");
        assert_eq!(f.kind, FailureKind::Internal);
        assert_eq!(f.reason, "from the future");
        assert_eq!(
            back.stats.total_checks, 41,
            "the rest of the message survives"
        );
    }

    /// `retries` is a `u32` on the wire and a `u8` in the snapshot. `as u8` would turn
    /// 256 into 0 — "no retries" — which reads as healthy.
    #[test]
    fn retries_beyond_a_byte_clamp_rather_than_wrap() {
        let mut msg = wire();
        msg.retries = 300;
        assert_eq!(RemoteStats::try_from(msg).unwrap().stats.retries, u8::MAX);
    }

    /// A plane of the wrong length can only come from a peer with a different bucket
    /// count, and every position in it would be pinned to the wrong five minutes.
    #[test]
    fn a_plane_of_the_wrong_length_is_dropped() {
        let mut msg = wire();
        msg.activity_ok = Bytes::from(vec![0xFF; ACTIVITY_PLANE_BYTES + 4]);
        msg.activity_fail = Bytes::from(vec![0xFF; 4]);
        msg.total_records = 9;

        let back = RemoteStats::try_from(msg).expect("a bad plane is not fatal");
        assert!(back.stats.activity_ok.is_empty(), "too long");
        assert!(back.stats.activity_fail.is_empty(), "too short");
        assert_eq!(back.stats.total_records, 9, "the counters still arrive");

        // A plane of exactly the right length passes through untouched.
        let mut ok = wire();
        ok.activity_ok = Bytes::from(vec![0xFF; ACTIVITY_PLANE_BYTES]);
        let back = RemoteStats::try_from(ok).unwrap();
        assert_eq!(back.stats.activity_ok.len(), ACTIVITY_PLANE_BYTES);
    }

    /// Bucket positions are relative to the window end, so an unusable end makes the
    /// planes unreadable rather than merely odd — and a plane read against the wrong
    /// end is worse than no plane, because it looks like data.
    #[test]
    fn an_unusable_window_end_drops_the_planes() {
        let mut msg = wire();
        msg.activity_ok = Bytes::from(vec![0xFF; ACTIVITY_PLANE_BYTES]);
        msg.activity_fail = Bytes::from(vec![0xFF; ACTIVITY_PLANE_BYTES]);
        msg.activity_window_end_unix_ms = i64::MAX;

        let back = RemoteStats::try_from(msg).expect("a bad window is not fatal");
        assert!(back.stats.activity_ok.is_empty());
        assert!(back.stats.activity_fail.is_empty());
    }

    /// A failure is a point on a timeline. One that can't be placed on it is dropped
    /// — including out of the middle of the ring, which must not shift the rest.
    #[test]
    fn a_failure_with_an_impossible_timestamp_is_dropped() {
        let bad = v1::ItemFailure {
            at_unix_ms: i64::MIN,
            kind: v1::FailureKind::Timeout as i32,
            reason: "unplaceable".into(),
            detail_json: Bytes::new(),
        };
        let good = v1::ItemFailure {
            at_unix_ms: at_bucket(5),
            kind: v1::FailureKind::Auth as i32,
            reason: "rejected".into(),
            detail_json: Bytes::new(),
        };

        let mut msg = wire();
        msg.last_failure = Some(bad.clone());
        msg.recent_failures = vec![bad, good];

        let back = RemoteStats::try_from(msg).unwrap();
        assert!(back.stats.last_failure.is_none());
        let ring = back.stats.recent_failures.expect("the good one survives");
        assert_eq!(ring.len(), 1);
        assert_eq!(ring[0].reason, "rejected");
    }

    /// The one place the conversion loses information, asserted so it stays a known
    /// loss: an empty ring is `None` whether the item never failed or the response
    /// excluded the ring. `Some(vec![])` would be a third state meaning neither.
    #[test]
    fn an_empty_ring_comes_back_absent() {
        let snap = StatsSnapshot {
            last_failure: Some(Arc::new(failure(FailureKind::Auth))),
            recent_failures: Some(Arc::new(vec![failure(FailureKind::Auth)])),
            ..Default::default()
        };

        let slim = to_proto(Uuid::now_v7(), Uuid::now_v7(), &snap, false);
        let back = RemoteStats::try_from(slim).unwrap();
        assert!(back.stats.recent_failures.is_none());
        assert!(
            back.stats.last_failure.is_some(),
            "the tell that the ring was excluded rather than empty"
        );
    }

    /// The wire carries an instant and no offset, so a received timestamp is rendered
    /// in the receiving node's zone — the same one this node's own `snapshot()` uses,
    /// so a merged fan-out doesn't show half its rows at a different offset.
    ///
    /// The offset half of that claim is only meaningfully checked by the test below:
    /// `DEFAULT_TZ` is UTC under an unset `$TZ`, and then this asserts `0 == 0`.
    #[test]
    fn a_received_timestamp_keeps_its_instant_exactly() {
        use chrono::Offset;

        let ms = at_bucket(100);
        let mut msg = wire();
        msg.last_success_unix_ms = Some(ms);

        let back = RemoteStats::try_from(msg).unwrap();
        let got = back.stats.last_success.expect("succeeded");
        assert_eq!(got.timestamp_millis(), ms, "the instant is exact");
        assert_eq!(
            *got.offset(),
            clock(ms)
                .with_timezone(&*crate::common::DEFAULT_TZ)
                .offset()
                .fix(),
            "rendered in the node's zone"
        );
    }

    /// The zone is resolved per instant rather than stamped from "now", so a timestamp
    /// from the other side of a DST change carries the offset that was in force
    /// *then*.
    ///
    /// Explicit zones rather than `DEFAULT_TZ`, so this holds whatever `$TZ` the test
    /// runner has — and so that dropping the zone entirely (rendering everything in
    /// UTC) actually fails something.
    #[test]
    fn a_timestamp_is_rendered_at_the_offset_in_force_for_that_instant() {
        use chrono::Offset;

        let ms = |rfc3339: &str| {
            DateTime::parse_from_rfc3339(rfc3339)
                .expect("valid timestamp")
                .timestamp_millis()
        };
        let offset_secs = |t: Option<DateTime<FixedOffset>>| {
            t.expect("in range").offset().fix().local_minus_utc()
        };

        // A zone that observes DST: the same wall-clock hour is -05:00 in January and
        // -04:00 in July.
        let ny = chrono_tz::America::New_York;
        assert_eq!(
            offset_secs(to_local_in(ms("2026-01-15T12:00:00Z"), &ny)),
            -5 * 3600
        );
        assert_eq!(
            offset_secs(to_local_in(ms("2026-07-15T12:00:00Z"), &ny)),
            -4 * 3600
        );

        // A zone that doesn't: one offset, always, and never UTC.
        let jakarta = chrono_tz::Asia::Jakarta;
        assert_eq!(
            offset_secs(to_local_in(ms("2026-01-15T12:00:00Z"), &jakarta)),
            7 * 3600
        );
    }

    // --- the answering sweep ---

    /// The instant every sweep test reads at unless it says otherwise, and the
    /// `stats_since` it reports. Far from the epoch so nothing passes on a zero.
    const SINCE_MS: i64 = 1_700_000_000_000;

    /// An unfiltered sweep over `items`, shaped as the RPC method builds it.
    fn sweep(items: Vec<(Uuid, Arc<Stats>)>) -> Sweep {
        Sweep {
            node_id: Uuid::now_v7(),
            now: clock(SINCE_MS),
            stats_since_unix_ms: SINCE_MS,
            include_recent_failures: true,
            states: StateFilter::new(&[]),
            items,
            unknown_item_ids: Vec::new(),
        }
    }

    async fn chunks(s: Sweep) -> Vec<v1::ItemStatsResponse> {
        use futures::StreamExt;
        s.into_stream().collect().await
    }

    /// Every `ItemStats` across every chunk, in order.
    fn flatten(out: &[v1::ItemStatsResponse]) -> Vec<&v1::ItemStats> {
        out.iter().flat_map(|c| &c.stats).collect()
    }

    /// An item whose last check succeeded — stored state `Ok`.
    fn ok_item() -> (Uuid, Arc<Stats>) {
        let stats = Stats::default();
        stats.record_attempt(SINCE_MS, std::time::Duration::ZERO);
        stats.record_success(SINCE_MS, std::time::Duration::from_millis(5), 3);
        (Uuid::now_v7(), Arc::new(stats))
    }

    /// A failing item carrying `blob` bytes of failure detail, for driving the
    /// chunk budget with a realistic payload rather than a synthetic one.
    fn failing_item(blob: usize) -> (Uuid, Arc<Stats>) {
        let stats = Stats::default();
        stats.record_failure(
            Failure::new(
                clock(SINCE_MS),
                FailureKind::TargetError,
                "bulky",
                Some(json!({ "blob": "x".repeat(blob) })),
            ),
            std::time::Duration::ZERO,
            0,
            false,
        );
        (Uuid::now_v7(), Arc::new(stats))
    }

    /// Enough detail that two items can't share a chunk.
    const HALF_BUDGET_BLOB: usize = CHUNK_TARGET_BYTES / 2;

    /// A caller that gets no chunk at all can't tell a filter that matched nothing
    /// from a peer that never answered, and never learns `stats_since` either.
    #[tokio::test]
    async fn a_sweep_of_nothing_still_answers_once() {
        let s = sweep(Vec::new());
        let node_id = s.node_id;
        let out = chunks(s).await;

        assert_eq!(out.len(), 1);
        assert!(out[0].stats.is_empty());
        assert_eq!(out[0].node_id.as_ref(), node_id.as_bytes());
        assert_eq!(out[0].stats_since_unix_ms, SINCE_MS);
    }

    /// Interpreting a chunk must not require having kept the first one.
    #[tokio::test]
    async fn every_chunk_names_the_node_and_the_since() {
        let s = sweep((0..3).map(|_| failing_item(HALF_BUDGET_BLOB)).collect());
        let node_id = s.node_id;
        let out = chunks(s).await;

        assert!(
            out.len() > 1,
            "expected a split, got {} chunk(s)",
            out.len()
        );
        for c in &out {
            assert_eq!(c.node_id.as_ref(), node_id.as_bytes());
            assert_eq!(c.stats_since_unix_ms, SINCE_MS);
        }
    }

    #[tokio::test]
    async fn only_the_last_chunk_names_unknown_ids() {
        let missing = vec![Uuid::now_v7(), Uuid::now_v7()];
        let mut s = sweep((0..3).map(|_| failing_item(HALF_BUDGET_BLOB)).collect());
        s.unknown_item_ids = missing.clone();
        let out = chunks(s).await;

        assert!(
            out.len() > 1,
            "expected a split, got {} chunk(s)",
            out.len()
        );
        let (last, rest) = out.split_last().expect("always at least one chunk");
        for c in rest {
            assert!(c.unknown_item_ids.is_empty());
        }
        let got: Vec<Uuid> = last
            .unknown_item_ids
            .iter()
            .map(|b| Uuid::from_slice(b).expect("uuid round-trips"))
            .collect();
        assert_eq!(got, missing);
    }

    #[tokio::test]
    async fn an_empty_state_filter_matches_everything() {
        let out = chunks(sweep(vec![ok_item(), failing_item(8)])).await;
        assert_eq!(flatten(&out).len(), 2);
    }

    #[tokio::test]
    async fn a_state_filter_keeps_only_what_it_names() {
        let mut s = sweep(vec![ok_item(), failing_item(8)]);
        s.states = StateFilter::new(&[v1::ItemState::Failing as i32]);
        let out = chunks(s).await;

        let all = flatten(&out);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].state, v1::ItemState::Failing as i32);
    }

    /// A state only a newer peer knows about can't match anything here — but it
    /// must not be mistaken for "no filter", which would answer a narrow question
    /// with the entire fleet.
    #[tokio::test]
    async fn an_unrecognised_state_matches_nothing() {
        let mut s = sweep(vec![ok_item(), failing_item(8)]);
        s.states = StateFilter::new(&[99]);
        let out = chunks(s).await;

        assert!(flatten(&out).is_empty());
        assert_eq!(out.len(), 1, "still owes the caller one chunk");
    }

    /// The filter runs against the snapshot's state, not the stored byte.
    ///
    /// `Stale` is derived on read — an item is stale precisely because nothing is
    /// executing it, so no writer can ever store it. Filtering on the atomic would
    /// report this item as `Ok` and miss every silent item in the fleet, which is
    /// the most interesting query this RPC serves.
    #[tokio::test]
    async fn a_stale_item_answers_a_stale_filter_and_not_its_stored_state() {
        let stats = Stats::default();
        stats.record_attempt(SINCE_MS, std::time::Duration::ZERO);
        stats.record_success(SINCE_MS, std::time::Duration::from_millis(5), 1);
        stats.set_next_check(SINCE_MS + 1_000, SINCE_MS + 2_000);
        let item = (Uuid::now_v7(), Arc::new(stats));
        let past_the_deadline = clock(SINCE_MS + 5_000);

        let mut s = sweep(vec![item.clone()]);
        s.now = past_the_deadline;
        s.states = StateFilter::new(&[v1::ItemState::Stale as i32]);
        let out = chunks(s).await;
        let all = flatten(&out);
        assert_eq!(all.len(), 1, "a stale item must answer a stale filter");
        assert_eq!(all[0].state, v1::ItemState::Stale as i32);

        let mut s = sweep(vec![item]);
        s.now = past_the_deadline;
        s.states = StateFilter::new(&[v1::ItemState::Ok as i32]);
        let out = chunks(s).await;
        assert!(
            flatten(&out).is_empty(),
            "the state it was stored as no longer applies"
        );
    }

    /// A chunk over budget is allowed only when a single item put it there, and no
    /// item may be lost to the splitting.
    #[tokio::test]
    async fn chunks_stay_within_the_budget() {
        let out = chunks(sweep(
            (0..4).map(|_| failing_item(HALF_BUDGET_BLOB)).collect(),
        ))
        .await;

        assert!(out.len() > 1, "the budget never forced a flush");
        assert_eq!(flatten(&out).len(), 4, "the split dropped an item");
        for c in &out {
            assert!(
                c.encoded_len() <= CHUNK_TARGET_BYTES || c.stats.len() == 1,
                "{} bytes across {} items",
                c.encoded_len(),
                c.stats.len(),
            );
        }
    }

    /// An item too big for a whole chunk still gets reported. It produces an
    /// oversized message, which is the lesser evil: the alternative is dropping a
    /// broken item's diagnostics precisely because they are detailed.
    #[tokio::test]
    async fn an_item_larger_than_the_budget_ships_alone() {
        let out = chunks(sweep(vec![failing_item(CHUNK_TARGET_BYTES + 4096)])).await;

        // One chunk, not an empty one flushed ahead of it: the flush is gated on the
        // batch having something in it.
        assert_eq!(out.len(), 1);
        let all = flatten(&out);
        assert_eq!(all.len(), 1);
        assert!(all[0].encoded_len() > CHUNK_TARGET_BYTES);
    }

    #[tokio::test]
    async fn excluding_recent_failures_applies_to_every_item() {
        let mut s = sweep((0..3).map(|_| failing_item(8)).collect());
        s.include_recent_failures = false;
        let out = chunks(s).await;

        let all = flatten(&out);
        assert_eq!(all.len(), 3);
        for st in all {
            assert!(st.recent_failures.is_empty());
            assert!(st.last_failure.is_some(), "the latest one still travels");
        }
    }

    #[test]
    fn a_malformed_request_id_is_rejected() {
        let good = Uuid::now_v7();
        let parsed = parse_ids(&[Bytes::from(good.as_bytes().to_vec())]).expect("well-formed id");
        assert_eq!(parsed, vec![good]);

        let err = parse_ids(&[Bytes::from_static(&[1, 2, 3, 4])]).expect_err("4 bytes is no uuid");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    // --- folding one node's stream back into snapshots ---

    /// A peer's stream, as it arrives on the wire: exactly what the serving side
    /// produces for the same sweep. Nothing here fakes the chunking.
    fn as_peer_stream(s: Sweep) -> impl Stream<Item = Result<v1::ItemStatsResponse, Status>> {
        s.into_stream().map(Ok)
    }

    /// The whole path, server to client, with no gRPC in it.
    ///
    /// `Sweep::into_stream` emits precisely the chunks a peer sends and `collect_node`
    /// consumes precisely what a peer's stream yields, so joining them covers the seam
    /// neither half could reach alone. Compares whole `StatsSnapshot`s, so a field that
    /// stops surviving the crossing fails here without anyone extending the test.
    #[tokio::test]
    async fn a_node_answer_survives_the_round_trip() {
        let items = vec![
            ok_item(),
            failing_item(64),
            (Uuid::now_v7(), Arc::new(Stats::default())),
        ];
        let expected: Vec<(Uuid, StatsSnapshot)> = items
            .iter()
            .map(|(id, s)| (*id, s.snapshot_at(clock(SINCE_MS))))
            .collect();

        let s = sweep(items);
        let node_id = s.node_id;
        let got = collect_node(node_id, as_peer_stream(s)).await;

        assert!(got.error.is_none(), "{:?}", got.error);
        assert_eq!(got.node_id, node_id);
        assert_eq!(got.stats.len(), expected.len());
        for (id, want) in expected {
            assert_eq!(
                got.stats.get(&id),
                Some(&want),
                "item {id} came back changed"
            );
        }
    }

    #[tokio::test]
    async fn every_item_arrives_across_a_chunk_split() {
        let items: Vec<_> = (0..4).map(|_| failing_item(HALF_BUDGET_BLOB)).collect();
        let ids: Vec<Uuid> = items.iter().map(|(id, _)| *id).collect();

        // Confirm the input really does span chunks, or this proves nothing.
        let split = chunks(sweep(items.clone())).await;
        assert!(split.len() > 1, "input did not span chunks");

        let s = sweep(items);
        let node_id = s.node_id;
        let got = collect_node(node_id, as_peer_stream(s)).await;
        assert!(got.error.is_none());
        for id in ids {
            assert!(got.stats.contains_key(&id), "item {id} lost at a boundary");
        }
    }

    #[tokio::test]
    async fn the_node_and_since_come_off_the_first_chunk() {
        let s = sweep(vec![ok_item()]);
        let node_id = s.node_id;
        let got = collect_node(node_id, as_peer_stream(s)).await;

        assert_eq!(got.node_id, node_id);
        assert_eq!(
            got.since.map(|t| t.timestamp_millis()),
            Some(SINCE_MS),
            "since must survive the crossing as the same instant"
        );
    }

    /// Half a node's stats plus the reason for the other half beats discarding both.
    #[tokio::test]
    async fn a_broken_stream_keeps_what_arrived() {
        let s = sweep(vec![ok_item()]);
        let node_id = s.node_id;
        let first = chunks(s).await.into_iter().next().expect("one chunk");
        let delivered: Vec<Uuid> = first
            .stats
            .iter()
            .map(|st| Uuid::from_slice(&st.item_id).expect("uuid"))
            .collect();

        let broken =
            futures::stream::iter(vec![Ok(first), Err(Status::unavailable("peer went away"))]);
        let got = collect_node(node_id, broken).await;

        assert!(got.error.is_some(), "the break must be reported");
        assert!(got.since.is_some(), "it identified itself before breaking");
        assert_eq!(got.stats.len(), delivered.len());
        for id in delivered {
            assert!(got.stats.contains_key(&id));
        }
    }

    /// The server owes one chunk even when nothing matched, so silence is a fault —
    /// reporting it as an empty success would read as "this node has no items".
    #[tokio::test]
    async fn a_stream_that_says_nothing_is_an_error() {
        let empty = futures::stream::empty::<Result<v1::ItemStatsResponse, Status>>();
        let got = collect_node(Uuid::now_v7(), empty).await;

        assert!(got.error.is_some());
        assert!(got.since.is_none(), "it never told us its epoch");
        assert!(got.stats.is_empty());
    }

    #[tokio::test]
    async fn an_unconvertible_item_does_not_sink_the_node() {
        let s = sweep(vec![ok_item(), failing_item(8)]);
        let node_id = s.node_id;
        let mut chunk = chunks(s).await.into_iter().next().expect("one chunk");
        assert_eq!(chunk.stats.len(), 2);
        // Only a malformed identity reaches `collect_node` as an error at all; every
        // other field has a degraded reading and takes it.
        chunk.stats[0].item_id = Bytes::from_static(&[9, 9]);
        let survivor = Uuid::from_slice(&chunk.stats[1].item_id).expect("uuid");

        let got = collect_node(node_id, futures::stream::iter(vec![Ok(chunk)])).await;

        assert!(got.error.is_none(), "one bad item is not a bad node");
        assert_eq!(got.stats.len(), 1);
        assert!(got.stats.contains_key(&survivor));
    }

    // --- this node's own contribution ---

    /// The local shortcut must be indistinguishable from asking over the wire, or the
    /// element for this node would differ in kind from every other element.
    #[tokio::test]
    async fn the_local_path_matches_what_a_peer_would_have_sent() {
        let items = vec![ok_item(), failing_item(64)];
        let s = sweep(items.clone());
        let node_id = s.node_id;

        let remote = collect_node(node_id, as_peer_stream(s)).await;
        let local = merge_local(
            node_id,
            clock(SINCE_MS),
            clock(SINCE_MS),
            items,
            &StateFilter::new(&[]),
        );

        assert_eq!(local.node_id, remote.node_id);
        assert_eq!(local.since, remote.since);
        assert_eq!(local.stats, remote.stats);
    }

    #[tokio::test]
    async fn the_local_path_honours_the_state_filter() {
        let items = vec![ok_item(), failing_item(8)];
        let got = merge_local(
            Uuid::now_v7(),
            clock(SINCE_MS),
            clock(SINCE_MS),
            items,
            &StateFilter::new(&[v1::ItemState::Failing as i32]),
        );

        assert_eq!(got.stats.len(), 1);
        assert!(
            got.stats.values().all(|s| s.state == ItemState::Failing),
            "the filter let through something it shouldn't"
        );
    }
}
