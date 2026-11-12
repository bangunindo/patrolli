// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The DNS answers the `dns_lookup` processor reads, and the queue that fills
//! them.
//!
//! The processor hot path is a sync fn on plain OS threads, so a lookup only
//! ever reads memory: a fresh entry is answered, a stale one is answered and
//! queued for a refresh, a missing one returns nothing and is queued. The
//! resolver (`patrolli_dns`) takes questions off the queue and hands the
//! outcome back through [`Dns::complete`], so the next record finds it.
//!
//! Everything here is keyed by [`Question`] and never looks inside it, so a new
//! record type is a variant there, a variant in [`Answer`] and an arm in the
//! resolver.
//!
//! A question is queued once until it is resolved: a hot uncached address
//! would otherwise fill the queue with itself. The mark is set before the send
//! and cleared after the answer is stored (or by [`Pending`]'s drop, when a
//! lookup is aborted), so a concurrent lookup always sees one or the other.
//!
//! Per node, never shared across the cluster.

use crate::common;
use crate::common::internal_metrics::{Dns as DnsSnapshot, DnsRecord};
use crate::config::dns::{Dns as DnsConfig, DnsMode, TtlPolicy};
use anyhow::{Result, bail};
use flume::{Receiver, Sender, TrySendError};
use ipnet::IpNet;
use moka::Expiry;
use rustc_hash::FxBuildHasher;
use smol_str::SmolStr;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, Instant};

pub(super) static DNS_CACHE: LazyLock<Arc<Dns>> = LazyLock::new(|| Arc::new(Dns::default()));

/// What is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Question {
    /// the name of an address (`in-addr.arpa` / `ip6.arpa`)
    Ptr(IpAddr),
}

impl Question {
    pub fn record(&self) -> Record {
        match self {
            Question::Ptr(_) => Record::Ptr,
        }
    }
}

/// A record type, for per-type counters and config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Record {
    Ptr,
}

impl Record {
    pub const ALL: [Record; 1] = [Record::Ptr];

    pub fn as_str(self) -> &'static str {
        match self {
            Record::Ptr => "ptr",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// What a question is answered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// without the trailing dot. `SmolStr`, so handing it to every record
    /// that asks is a copy of the inline bytes or an `Arc` bump, never an
    /// allocation.
    Name(SmolStr),
}

/// The result of resolving one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Found {
        answer: Answer,
        ttl: Duration,
    },
    /// The name does not exist or has no such record; `ttl` comes from the
    /// SOA, when there is one.
    Negative {
        ttl: Option<Duration>,
    },
    /// Timeout or any other error: says nothing about whether a record exists.
    Failed,
}

#[derive(Debug, Clone)]
struct Entry {
    answer: Option<Answer>,
    /// a real DNS answer (a name, or "no such name"), not a failure
    answered: bool,
    /// [`now_ms`] until which the entry is served as is; it is stale after
    fresh_until: u64,
    /// [`now_ms`] at which it is gone
    evict_at: u64,
}

struct EvictAt;

impl EvictAt {
    fn remaining(entry: &Entry) -> Option<Duration> {
        Some(Duration::from_millis(
            entry.evict_at.saturating_sub(now_ms()),
        ))
    }
}

impl Expiry<Question, Entry> for EvictAt {
    fn expire_after_create(&self, _: &Question, entry: &Entry, _: Instant) -> Option<Duration> {
        Self::remaining(entry)
    }

    /// The default keeps the old deadline, which would evict a refreshed
    /// entry on its predecessor's schedule.
    fn expire_after_update(
        &self,
        _: &Question,
        entry: &Entry,
        _: Instant,
        _: Option<Duration>,
    ) -> Option<Duration> {
        Self::remaining(entry)
    }
}

/// Milliseconds since the process started, the clock entries are stamped in.
pub fn now_ms() -> u64 {
    common::uptime().as_millis() as u64
}

#[derive(Default)]
struct RecordCounters {
    lookups_hit: AtomicU64,
    lookups_stale: AtomicU64,
    lookups_missed: AtomicU64,
    lookups_skipped: AtomicU64,
    resolved_positive: AtomicU64,
    resolved_negative: AtomicU64,
    resolved_failed: AtomicU64,
}

#[derive(Default)]
struct Metrics {
    records: [RecordCounters; Record::ALL.len()],
    queue_enqueued: AtomicU64,
    queue_dropped: AtomicU64,
}

struct Inner {
    store: moka::sync::Cache<Question, Entry, FxBuildHasher>,
    /// queued or being resolved
    pending: papaya::HashSet<Question, FxBuildHasher>,
    tx: Sender<Question>,
    rx: Receiver<Question>,
    ttl: TtlPolicy,
    /// Recursive mode: an internal address outside every zone has nobody to
    /// answer it, so it is never queued.
    skip_internal: bool,
    zones: Vec<IpNet>,
    max_entries: u64,
    queue_capacity: usize,
}

impl Inner {
    fn new(config: &DnsConfig) -> Self {
        let (tx, rx) = flume::bounded(config.queue_capacity);
        Self {
            store: moka::sync::Cache::builder()
                .max_capacity(config.max_entries)
                .expire_after(EvictAt)
                .build_with_hasher(FxBuildHasher),
            pending: papaya::HashSet::with_hasher(FxBuildHasher),
            tx,
            rx,
            ttl: config.ttl_policy(),
            skip_internal: config.mode == DnsMode::Recursive,
            zones: config.zone_cidrs(),
            max_entries: config.max_entries,
            queue_capacity: config.queue_capacity,
        }
    }

    fn skipped(&self, question: &Question) -> bool {
        match question {
            Question::Ptr(ip) => {
                self.skip_internal
                    && is_internal(*ip)
                    && !self.zones.iter().any(|zone| zone.contains(ip))
            }
        }
    }
}

/// Addresses the public DNS tree cannot name: private, shared (CGNAT),
/// loopback, link-local and unspecified ones.
pub fn is_internal(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
        }
    }
}

/// The cache. [`Dns::default`] (the process singleton) takes its settings on
/// the first [`Dns::configure`], or the defaults on first use.
#[derive(Default)]
pub struct Dns {
    inner: OnceLock<Inner>,
    metrics: Metrics,
}

impl Dns {
    pub fn new(config: &DnsConfig) -> Self {
        let dns = Self::default();
        let _ = dns.inner.set(Inner::new(config));
        dns
    }

    /// Size the singleton before anything reads it. Once only: the queue and
    /// the store cannot be swapped under a running resolver.
    pub fn configure(&self, config: &DnsConfig) -> Result<()> {
        if self.inner.set(Inner::new(config)).is_err() {
            bail!("the DNS cache is already configured");
        }
        Ok(())
    }

    fn inner(&self) -> &Inner {
        self.inner.get_or_init(|| Inner::new(&DnsConfig::default()))
    }

    /// A reader for one batch: counts locally and adds to the shared
    /// counters once, when dropped.
    pub fn reader(&self) -> Reader<'_> {
        Reader {
            dns: self,
            inner: self.inner(),
            now: now_ms(),
            counts: Default::default(),
            counted: true,
        }
    }

    /// A reader that leaves the lookup counters alone, for a processor
    /// preview. Misses and stale entries are still queued, so a preview run
    /// again finds what the first one asked.
    pub fn uncounted_reader(&self) -> Reader<'_> {
        Reader {
            counted: false,
            ..self.reader()
        }
    }

    /// Where the resolver takes questions from.
    pub fn requests(&self) -> Receiver<Question> {
        self.inner().rx.clone()
    }

    /// Mark a question taken off the queue as in flight. The mark is cleared
    /// by [`Pending::complete`], or by the drop if the lookup never finishes.
    pub fn pending(self: &Arc<Self>, question: Question) -> Pending {
        Pending {
            dns: Arc::clone(self),
            question: Some(question),
        }
    }

    /// Store the outcome of resolving `question` and clear its pending mark.
    ///
    /// TTLs are clamped by the configured policy. A failure never replaces a
    /// real answer still inside its stale window: the last answer stays, and
    /// is asked again after `failure_ttl`, until the window ends.
    pub fn complete(&self, question: Question, outcome: Outcome) {
        let inner = self.inner();
        let ttl = &inner.ttl;
        let now = now_ms();
        let counters = &self.metrics.records[question.record().index()];
        let entry = |answer, answered, fresh: Duration| {
            let fresh_until = now.saturating_add(fresh.as_millis() as u64);
            Entry {
                answer,
                answered,
                fresh_until,
                evict_at: fresh_until.saturating_add(ttl.stale.as_millis() as u64),
            }
        };
        let entry = match outcome {
            Outcome::Found { answer, ttl: found } => {
                add(&counters.resolved_positive, 1);
                entry(
                    Some(answer),
                    true,
                    found.clamp(ttl.positive_min, ttl.positive_max),
                )
            }
            Outcome::Negative { ttl: negative } => {
                add(&counters.resolved_negative, 1);
                entry(
                    None,
                    true,
                    negative
                        .unwrap_or(ttl.negative_default)
                        .clamp(ttl.negative_min, ttl.negative_max),
                )
            }
            Outcome::Failed => {
                add(&counters.resolved_failed, 1);
                match inner.store.get(&question) {
                    Some(last) if last.answered && last.evict_at > now => Entry {
                        fresh_until: now
                            .saturating_add(ttl.failure.as_millis() as u64)
                            .min(last.evict_at),
                        ..last
                    },
                    _ => entry(None, false, ttl.failure),
                }
            }
        };
        // Stored before the mark goes, so a lookup in between finds one of the
        // two and does not queue the question again.
        inner.store.insert(question, entry);
        inner.pending.pin().remove(&question);
    }

    fn enqueue(&self, inner: &Inner, question: Question) {
        if !inner.pending.pin().insert(question) {
            return;
        }
        match inner.tx.try_send(question) {
            Ok(()) => add(&self.metrics.queue_enqueued, 1),
            // The receiver lives in `inner`, so the queue only ever fills.
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                inner.pending.pin().remove(&question);
                add(&self.metrics.queue_dropped, 1);
            }
        }
    }

    /// The cache's half of the `dns` snapshot; the resolver fills in the rest.
    pub fn snapshot(&self) -> DnsSnapshot {
        let inner = self.inner();
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        DnsSnapshot {
            records: Record::ALL
                .iter()
                .map(|record| {
                    let c = &self.metrics.records[record.index()];
                    DnsRecord {
                        record: record.as_str(),
                        lookups_hit: load(&c.lookups_hit),
                        lookups_stale: load(&c.lookups_stale),
                        lookups_missed: load(&c.lookups_missed),
                        lookups_skipped: load(&c.lookups_skipped),
                        resolved_positive: load(&c.resolved_positive),
                        resolved_negative: load(&c.resolved_negative),
                        resolved_failed: load(&c.resolved_failed),
                    }
                })
                .collect(),
            queue_enqueued: load(&self.metrics.queue_enqueued),
            queue_dropped: load(&self.metrics.queue_dropped),
            queue_length: inner.rx.len() as u64,
            queue_capacity: inner.queue_capacity as u64,
            cache_entries: inner.store.entry_count(),
            cache_max_entries: inner.max_entries,
            ..Default::default()
        }
    }
}

#[derive(Default, Clone, Copy)]
struct LookupCounts {
    hit: u64,
    stale: u64,
    missed: u64,
    skipped: u64,
}

/// One batch's view of the cache, at one point in time.
pub struct Reader<'a> {
    dns: &'a Dns,
    inner: &'a Inner,
    now: u64,
    counts: [LookupCounts; Record::ALL.len()],
    counted: bool,
}

impl Reader<'_> {
    /// The cached answer, or `None` for a miss, a negative or failed entry, or
    /// a skipped question. Never waits: a miss or a stale entry is queued.
    pub fn lookup(&mut self, question: &Question) -> Option<Answer> {
        let counts = &mut self.counts[question.record().index()];
        if self.inner.skipped(question) {
            counts.skipped += 1;
            return None;
        }
        match self.inner.store.get(question) {
            Some(entry) if self.now < entry.fresh_until => {
                counts.hit += 1;
                entry.answer
            }
            Some(entry) if self.now < entry.evict_at => {
                counts.stale += 1;
                self.dns.enqueue(self.inner, *question);
                entry.answer
            }
            _ => {
                counts.missed += 1;
                self.dns.enqueue(self.inner, *question);
                None
            }
        }
    }
}

impl Drop for Reader<'_> {
    fn drop(&mut self) {
        if !self.counted {
            return;
        }
        for (counts, shared) in self.counts.iter().zip(&self.dns.metrics.records) {
            add(&shared.lookups_hit, counts.hit);
            add(&shared.lookups_stale, counts.stale);
            add(&shared.lookups_missed, counts.missed);
            add(&shared.lookups_skipped, counts.skipped);
        }
    }
}

/// A question being resolved. Dropping it unfinished (an aborted or panicked
/// lookup) clears the pending mark, so the question can be queued again.
pub struct Pending {
    dns: Arc<Dns>,
    question: Option<Question>,
}

impl Pending {
    pub fn question(&self) -> Question {
        self.question.expect("taken only by complete")
    }

    pub fn complete(mut self, outcome: Outcome) {
        if let Some(question) = self.question.take() {
            self.dns.complete(question, outcome);
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(question) = self.question.take() {
            self.dns.inner().pending.pin().remove(&question);
        }
    }
}

fn add(counter: &AtomicU64, n: u64) {
    if n != 0 {
        counter.fetch_add(n, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ptr(ip: &str) -> Question {
        Question::Ptr(ip.parse().unwrap())
    }

    fn found(name: &str, ttl: Duration) -> Outcome {
        Outcome::Found {
            answer: Answer::Name(name.into()),
            ttl,
        }
    }

    fn name(answer: Option<Answer>) -> Option<String> {
        answer.map(|Answer::Name(name)| name.to_string())
    }

    fn config(edit: impl FnOnce(&mut DnsConfig)) -> DnsConfig {
        let mut config = DnsConfig::default();
        edit(&mut config);
        config
    }

    fn lookup(dns: &Dns, question: &Question) -> Option<String> {
        name(dns.reader().lookup(question))
    }

    fn snapshot_record(dns: &Dns) -> DnsRecord {
        dns.snapshot().records[0].clone()
    }

    #[test]
    fn a_miss_is_queued_once_and_a_later_lookup_hits() {
        let dns = Arc::new(Dns::new(&DnsConfig::default()));
        let requests = dns.requests();
        let q = ptr("8.8.8.8");
        // many lookups of one uncached address, across batches
        for _ in 0..3 {
            let mut reader = dns.reader();
            for _ in 0..100 {
                assert_eq!(reader.lookup(&q), None);
            }
        }
        assert_eq!(requests.len(), 1, "one question for one address");
        assert_eq!(requests.try_recv().unwrap(), q);

        dns.pending(q)
            .complete(found("dns.google", Duration::from_secs(60)));
        assert_eq!(lookup(&dns, &q).as_deref(), Some("dns.google"));
        assert!(requests.is_empty());

        let record = snapshot_record(&dns);
        assert_eq!((record.lookups_missed, record.lookups_hit), (300, 1));
        assert_eq!(record.resolved_positive, 1);
        let snapshot = dns.snapshot();
        assert_eq!((snapshot.queue_enqueued, snapshot.queue_dropped), (1, 0));
    }

    #[test]
    fn an_aborted_lookup_lets_the_question_be_queued_again() {
        let dns = Arc::new(Dns::new(&DnsConfig::default()));
        let requests = dns.requests();
        let q = ptr("8.8.4.4");
        lookup(&dns, &q);
        let taken = requests.try_recv().unwrap();
        lookup(&dns, &q);
        assert!(requests.is_empty(), "still pending");
        drop(dns.pending(taken));
        lookup(&dns, &q);
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn a_full_queue_drops_without_blocking() {
        let dns = Dns::new(&config(|c| c.queue_capacity = 2));
        let requests = dns.requests();
        let mut reader = dns.reader();
        for i in 1..=5 {
            assert_eq!(reader.lookup(&ptr(&format!("1.1.1.{i}"))), None);
        }
        drop(reader);
        assert_eq!(requests.len(), 2);
        let snapshot = dns.snapshot();
        assert_eq!((snapshot.queue_enqueued, snapshot.queue_dropped), (2, 3));
        assert_eq!((snapshot.queue_length, snapshot.queue_capacity), (2, 2));
        // a dropped question is not marked pending: it is queued once there
        // is room
        requests.try_recv().unwrap();
        lookup(&dns, &ptr("1.1.1.5"));
        assert_eq!(requests.len(), 2);
    }

    #[test]
    fn a_stale_entry_is_served_and_refreshed_once() {
        let dns = Arc::new(Dns::new(&config(|c| {
            c.serve_stale = Duration::from_secs(60);
        })));
        let requests = dns.requests();
        let q = ptr("9.9.9.9");
        dns.complete(q, found("old.example", Duration::ZERO));
        std::thread::sleep(Duration::from_millis(5));

        for _ in 0..5 {
            assert_eq!(lookup(&dns, &q).as_deref(), Some("old.example"));
        }
        assert_eq!(requests.len(), 1, "refreshed once");
        assert_eq!(snapshot_record(&dns).lookups_stale, 5);

        dns.pending(requests.try_recv().unwrap())
            .complete(found("new.example", Duration::from_secs(60)));
        assert_eq!(lookup(&dns, &q).as_deref(), Some("new.example"));
    }

    #[test]
    fn a_failed_refresh_keeps_the_last_answer_until_the_stale_window_ends() {
        let dns = Arc::new(Dns::new(&config(|c| {
            c.positive_ttl_max = Duration::ZERO;
            c.serve_stale = Duration::from_secs(1);
            c.failure_ttl = Duration::from_secs(60);
        })));
        let requests = dns.requests();
        let q = ptr("8.8.8.8");
        dns.complete(q, found("dns.google", Duration::from_secs(3600)));
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(lookup(&dns, &q).as_deref(), Some("dns.google"));
        dns.pending(requests.try_recv().unwrap())
            .complete(Outcome::Failed);

        // kept, and fresh again (until the window ends): no new refresh
        assert_eq!(lookup(&dns, &q).as_deref(), Some("dns.google"));
        assert!(requests.is_empty());
        assert_eq!(snapshot_record(&dns).resolved_failed, 1);

        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(lookup(&dns, &q), None, "evicted after the stale window");
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn negative_and_failure_entries_keep_their_own_ttls() {
        let dns = Dns::new(&config(|c| {
            c.serve_stale = Duration::ZERO;
            c.negative_ttl_default = Duration::from_millis(1500);
            c.failure_ttl = Duration::from_millis(300);
        }));
        let requests = dns.requests();
        let (negative, failed) = (ptr("192.0.2.1"), ptr("192.0.2.2"));
        dns.complete(negative, Outcome::Negative { ttl: None });
        dns.complete(failed, Outcome::Failed);
        assert_eq!(lookup(&dns, &negative), None);
        assert_eq!(lookup(&dns, &failed), None);
        assert!(requests.is_empty(), "both cached, as nothing");
        assert_eq!(snapshot_record(&dns).lookups_hit, 2);

        std::thread::sleep(Duration::from_millis(500));
        lookup(&dns, &negative);
        lookup(&dns, &failed);
        assert_eq!(requests.len(), 1, "only the failure expired");
        assert_eq!(requests.try_recv().unwrap(), failed);
    }

    #[test]
    fn ttls_are_clamped() {
        let dns = Dns::new(&config(|c| {
            c.serve_stale = Duration::ZERO;
            c.positive_ttl_min = Duration::from_secs(1);
            c.negative_ttl_max = Duration::ZERO;
        }));
        let requests = dns.requests();
        let (short, negative) = (ptr("198.51.100.1"), ptr("198.51.100.2"));
        dns.complete(short, found("short.example", Duration::ZERO));
        dns.complete(
            negative,
            Outcome::Negative {
                ttl: Some(Duration::from_secs(3600)),
            },
        );
        std::thread::sleep(Duration::from_millis(5));
        // raised to the minimum, so still fresh
        assert_eq!(lookup(&dns, &short).as_deref(), Some("short.example"));
        // cut to zero, so already gone
        assert_eq!(lookup(&dns, &negative), None);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests.try_recv().unwrap(), negative);
    }

    #[test]
    fn internal_addresses_without_a_zone_are_skipped_in_recursive_mode() {
        let zoned: DnsConfig = serde_yaml_ng::from_str(
            "zones: [{cidrs: [10.1.0.0/16], forwarders: [udp://10.0.0.53]}]",
        )
        .unwrap();
        let dns = Dns::new(&zoned);
        let requests = dns.requests();
        let mut reader = dns.reader();
        for ip in [
            "10.2.0.1",
            "192.168.1.1",
            "127.0.0.1",
            "169.254.1.1",
            "fd00::1",
            "fe80::1",
        ] {
            assert_eq!(reader.lookup(&ptr(ip)), None);
        }
        reader.lookup(&ptr("10.1.2.3")); // inside the zone
        reader.lookup(&ptr("8.8.8.8"));
        drop(reader);
        assert_eq!(requests.len(), 2);
        assert_eq!(snapshot_record(&dns).lookups_skipped, 6);

        // forward mode sends them to the forwarders like everything else
        let forward = Dns::new(&config(|c| {
            c.mode = DnsMode::Forward;
            c.forwarders = vec!["udp://10.0.0.53".parse().unwrap()];
        }));
        let requests = forward.requests();
        lookup(&forward, &ptr("192.168.1.1"));
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn the_singleton_is_configured_once() {
        let dns = Dns::default();
        dns.configure(&config(|c| c.queue_capacity = 7)).unwrap();
        assert_eq!(dns.snapshot().queue_capacity, 7);
        assert!(dns.configure(&DnsConfig::default()).is_err());
    }
}
