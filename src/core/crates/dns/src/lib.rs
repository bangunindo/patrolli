// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The resolver that fills `cache::dns`, in process. Kept in its own crate so
//! hickory stays out of the core one.
//!
//! ```text
//! cache.dns queue ──> run: max_inflight at once ──> Router ──> cache.dns.complete
//!                                                     ├─ zone covering the address ─> its forwarders
//!                                                     ├─ recursive: recursor ─(failed)─> forwarders
//!                                                     └─ forward:   forwarders
//! ```
//!
//! [`spawn`] starts it, always: it idles on the empty queue until a
//! `dns_lookup` processor asks something. It returns on cancel, aborting what
//! is in flight; their pending marks go with them, so nothing is left queued
//! forever.

mod forwarder;
mod outcome;
mod recursor;

pub use forwarder::{Forwarder, ForwarderList, Limits, Upstream};

use anyhow::Result;
use futures::future::BoxFuture;
use ipnet::IpNet;
use patrolli::cache::dns::{Dns, Outcome, Question};
use patrolli::common::internal_metrics::INTERNAL_METRICS;
use patrolli::config::dns::{Dns as DnsConfig, DnsMode, Forwarder as ForwarderUrl};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, info, info_span};

/// Something that answers questions: the recursor, a forwarder list, the
/// [`Router`] in front of them, or a fake in tests.
pub trait Resolve: Send + Sync + 'static {
    fn resolve<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Outcome>;
}

/// The resolver's half of the `dns` metrics; the cache keeps the rest.
#[derive(Default)]
pub struct Metrics {
    inflight: AtomicU64,
    resolve_nanos: AtomicU64,
    forwarder_fallbacks: AtomicU64,
}

/// Picks who answers a question.
///
/// An address inside a zone goes to that zone's forwarders only, the longest
/// prefix winning. Anything else goes to `primary`, under `primary_timeout`
/// when set; `fallback` is asked only when that fails, never after a real
/// answer, negative ones included.
pub struct Router {
    /// longest prefix first
    zones: Vec<(IpNet, Arc<dyn Resolve>)>,
    primary: Arc<dyn Resolve>,
    primary_timeout: Option<Duration>,
    fallback: Option<Arc<dyn Resolve>>,
    metrics: Arc<Metrics>,
}

impl Router {
    pub fn new(
        mut zones: Vec<(IpNet, Arc<dyn Resolve>)>,
        primary: Arc<dyn Resolve>,
        primary_timeout: Option<Duration>,
        fallback: Option<Arc<dyn Resolve>>,
        metrics: Arc<Metrics>,
    ) -> Self {
        zones.sort_by_key(|(net, _)| std::cmp::Reverse(net.prefix_len()));
        Self {
            zones,
            primary,
            primary_timeout,
            fallback,
            metrics,
        }
    }

    fn zone(&self, question: &Question) -> Option<&Arc<dyn Resolve>> {
        match question {
            Question::Ptr(ip) => self
                .zones
                .iter()
                .find(|(net, _)| net.contains(ip))
                .map(|(_, zone)| zone),
        }
    }
}

impl Resolve for Router {
    fn resolve<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Outcome> {
        Box::pin(async move {
            if let Some(zone) = self.zone(question) {
                return zone.resolve(question).await;
            }
            let primary = self.primary.resolve(question);
            let outcome = match self.primary_timeout {
                Some(limit) => tokio::time::timeout(limit, primary)
                    .await
                    .unwrap_or(Outcome::Failed),
                None => primary.await,
            };
            match (&outcome, &self.fallback) {
                (Outcome::Failed, Some(fallback)) => {
                    self.metrics
                        .forwarder_fallbacks
                        .fetch_add(1, Ordering::Relaxed);
                    fallback.resolve(question).await
                }
                _ => outcome,
            }
        })
    }
}

/// Build the resolver `config` describes and start it, with its metrics
/// reporter, until `token` is cancelled.
///
/// Fails only on a resolver that cannot be built at all; an `https://`
/// forwarder whose host name does not resolve yet is left out and retried.
pub fn spawn(
    cache: Arc<Dns>,
    config: &DnsConfig,
    metrics_interval: Duration,
    token: CancellationToken,
) -> Result<JoinHandle<()>> {
    let limits = Limits::of(config);
    let metrics = Arc::new(Metrics::default());
    // One forwarder per URL, shared by every list naming it.
    let mut forwarders: BTreeMap<String, Arc<Forwarder>> = BTreeMap::new();
    let mut hosts = JoinSet::new();
    let mut list = |urls: &[ForwarderUrl]| -> Result<Arc<dyn Resolve>> {
        let mut members = Vec::with_capacity(urls.len());
        for url in urls {
            let name = url.to_string();
            if let Some(forwarder) = forwarders.get(&name) {
                members.push(forwarder.clone());
                continue;
            }
            let (upstream, host) = forwarder::HickoryUpstream::new(url, &limits)?;
            if let Some(host) = host {
                hosts.spawn(forwarder::resolve_host(
                    upstream.clone(),
                    url.clone(),
                    host,
                    limits,
                    token.clone(),
                ));
            }
            let forwarder = Arc::new(Forwarder::new(
                name.clone(),
                Box::new(forwarder::Shared(upstream)),
                limits,
            ));
            forwarders.insert(name, forwarder.clone());
            members.push(forwarder);
        }
        Ok(Arc::new(ForwarderList::new(members)))
    };
    let main = (!config.forwarders.is_empty())
        .then(|| list(&config.forwarders))
        .transpose()?;
    let mut zones = Vec::new();
    for zone in &config.zones {
        let resolver = list(&zone.forwarders)?;
        zones.extend(zone.cidrs.iter().map(|cidr| (cidr.0, resolver.clone())));
    }
    let (primary, primary_timeout, fallback) = match config.mode {
        DnsMode::Recursive => (
            Arc::new(recursor::RecursorSource::new(config)?) as Arc<dyn Resolve>,
            Some(config.lookup_timeout),
            main,
        ),
        DnsMode::Forward => (
            main.expect("validation requires forwarders in forward mode"),
            None,
            None,
        ),
    };
    let router: Arc<dyn Resolve> = Arc::new(Router::new(
        zones,
        primary,
        primary_timeout,
        fallback,
        metrics.clone(),
    ));
    let forwarders: Vec<_> = forwarders.into_values().collect();
    let max_inflight = config.max_inflight;
    let mode = config.mode;
    info!(
        mode = ?mode,
        forwarders = forwarders.len(),
        zones = config.zones.len(),
        "dns resolver started"
    );

    let reporter = report(
        cache.clone(),
        metrics.clone(),
        forwarders,
        max_inflight,
        metrics_interval,
        token.clone(),
    );
    Ok(tokio::spawn(
        async move {
            tokio::join!(
                run(cache, router, max_inflight, metrics, token.clone()),
                reporter,
            );
            // the host-name tasks end on the same token
            hosts.join_all().await;
            info!("dns resolver stopped");
        }
        .instrument(info_span!("dns")),
    ))
}

/// Take questions off the queue and resolve them, `max_inflight` at once,
/// until cancelled. Holding a permit before taking a question leaves the
/// rest on the queue, where a full one drops new questions instead of growing.
pub async fn run(
    cache: Arc<Dns>,
    resolver: Arc<dyn Resolve>,
    max_inflight: usize,
    metrics: Arc<Metrics>,
    token: CancellationToken,
) {
    let requests = cache.requests();
    let permits = Arc::new(Semaphore::new(max_inflight));
    // dropped on return, which aborts what is still in flight
    let mut lookups = JoinSet::new();
    loop {
        while lookups.try_join_next().is_some() {}
        let permit = tokio::select! {
            _ = token.cancelled() => return,
            permit = permits.clone().acquire_owned() => permit.expect("never closed"),
        };
        let question = tokio::select! {
            _ = token.cancelled() => return,
            question = requests.recv_async() => match question {
                Ok(question) => question,
                Err(_) => return,
            },
        };
        let pending = cache.pending(question);
        let (resolver, metrics) = (resolver.clone(), metrics.clone());
        lookups.spawn(async move {
            let _permit = permit;
            metrics.inflight.fetch_add(1, Ordering::Relaxed);
            let started = Instant::now();
            let outcome = resolver.resolve(&pending.question()).await;
            metrics
                .resolve_nanos
                .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
            metrics.inflight.fetch_sub(1, Ordering::Relaxed);
            pending.complete(outcome);
        });
    }
}

/// Publish the `dns` snapshot every `interval` until cancelled.
async fn report(
    cache: Arc<Dns>,
    metrics: Arc<Metrics>,
    forwarders: Vec<Arc<Forwarder>>,
    max_inflight: usize,
    interval: Duration,
    token: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = token.cancelled() => return,
            _ = tokio::time::sleep(interval) => {}
        }
        let mut snapshot = cache.snapshot();
        snapshot.inflight = metrics.inflight.load(Ordering::Relaxed);
        snapshot.max_inflight = max_inflight as u64;
        snapshot.resolve_time = Duration::from_nanos(metrics.resolve_nanos.load(Ordering::Relaxed));
        snapshot.forwarder_fallbacks = metrics.forwarder_fallbacks.load(Ordering::Relaxed);
        snapshot.forwarders = forwarders.iter().map(|f| f.snapshot()).collect();
        INTERNAL_METRICS
            .dns
            .store(Arc::new((std::time::Instant::now(), snapshot)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patrolli::cache::dns::Answer;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    /// Canned answers per question; unknown ones fail. Counts calls and the
    /// most it ever had running at once.
    #[derive(Default)]
    struct Fake {
        answers: Mutex<Vec<(Question, Outcome)>>,
        calls: Mutex<Vec<Question>>,
        delay: Duration,
        running: AtomicUsize,
        most_running: AtomicUsize,
    }

    impl Fake {
        fn new(answers: &[(&str, Outcome)]) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers.iter().map(|(ip, o)| (q(ip), o.clone())).collect()),
                ..Default::default()
            })
        }

        fn calls(&self, ip: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|c| **c == q(ip))
                .count()
        }
    }

    impl Resolve for Fake {
        fn resolve<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Outcome> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(*question);
                let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
                self.most_running.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(self.delay).await;
                self.running.fetch_sub(1, Ordering::SeqCst);
                self.answers
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(k, _)| k == question)
                    .map_or(Outcome::Failed, |(_, o)| o.clone())
            })
        }
    }

    fn q(ip: &str) -> Question {
        Question::Ptr(ip.parse().unwrap())
    }

    fn found(name: &str) -> Outcome {
        Outcome::Found {
            answer: Answer::Name(name.into()),
            ttl: Duration::from_secs(60),
        }
    }

    fn router(
        zones: Vec<(&str, Arc<Fake>)>,
        primary: Arc<Fake>,
        fallback: Option<Arc<Fake>>,
    ) -> (Router, Arc<Metrics>) {
        let metrics = Arc::new(Metrics::default());
        let zones = zones
            .into_iter()
            .map(|(cidr, zone)| (cidr.parse().unwrap(), zone as Arc<dyn Resolve>))
            .collect();
        (
            Router::new(
                zones,
                primary,
                Some(Duration::from_millis(200)),
                fallback.map(|f| f as Arc<dyn Resolve>),
                metrics.clone(),
            ),
            metrics,
        )
    }

    #[tokio::test]
    async fn a_failure_falls_back_to_the_forwarders_and_a_negative_answer_does_not() {
        let negative = Outcome::Negative { ttl: None };
        let recursor = Fake::new(&[("192.0.2.1", negative.clone())]);
        let forwarders = Fake::new(&[
            ("192.0.2.1", found("should.not.be.used")),
            ("203.0.113.9", found("from.forwarder")),
        ]);
        let (router, metrics) = router(vec![], recursor, Some(forwarders.clone()));
        assert_eq!(router.resolve(&q("192.0.2.1")).await, negative);
        assert_eq!(
            router.resolve(&q("203.0.113.9")).await,
            found("from.forwarder")
        );
        assert_eq!(forwarders.calls("192.0.2.1"), 0);
        assert_eq!(forwarders.calls("203.0.113.9"), 1);
        assert_eq!(metrics.forwarder_fallbacks.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn a_recursor_timeout_falls_back_too() {
        let slow = Fake {
            delay: Duration::from_secs(5),
            ..Default::default()
        };
        let forwarders = Fake::new(&[("8.8.8.8", found("dns.google"))]);
        let (router, _) = router(vec![], Arc::new(slow), Some(forwarders));
        assert_eq!(router.resolve(&q("8.8.8.8")).await, found("dns.google"));
    }

    #[tokio::test]
    async fn an_address_in_a_zone_goes_only_to_that_zone_and_the_longest_prefix_wins() {
        let (wide, narrow) = (
            Fake::new(&[("10.1.2.3", found("wide")), ("10.9.0.1", found("wide"))]),
            Fake::new(&[("10.1.2.3", found("narrow"))]),
        );
        let (recursor, forwarders) = (Fake::new(&[]), Fake::new(&[]));
        let (router, _) = router(
            vec![
                ("10.0.0.0/8", wide.clone()),
                ("10.1.0.0/16", narrow.clone()),
            ],
            recursor.clone(),
            Some(forwarders.clone()),
        );
        assert_eq!(router.resolve(&q("10.1.2.3")).await, found("narrow"));
        assert_eq!(router.resolve(&q("10.9.0.1")).await, found("wide"));
        // a zone that fails does not fall through to the public resolvers
        assert_eq!(router.resolve(&q("10.1.9.9")).await, Outcome::Failed);
        assert_eq!(recursor.calls.lock().unwrap().len(), 0);
        assert_eq!(forwarders.calls.lock().unwrap().len(), 0);
        assert_eq!(wide.calls("10.1.2.3"), 0);
    }

    #[tokio::test]
    async fn run_fills_the_cache_within_max_inflight_and_returns_on_cancel() {
        let cache = Arc::new(Dns::new(&DnsConfig::default()));
        let answers: Vec<_> = (1..=20)
            .map(|i| (format!("8.8.8.{i}"), found(&format!("host{i}.example"))))
            .collect();
        let fake = Arc::new(Fake {
            answers: Mutex::new(answers.iter().map(|(ip, o)| (q(ip), o.clone())).collect()),
            delay: Duration::from_millis(20),
            ..Default::default()
        });
        {
            let mut reader = cache.reader();
            for (ip, _) in &answers {
                assert_eq!(reader.lookup(&q(ip)), None);
            }
        }
        let token = CancellationToken::new();
        let metrics = Arc::new(Metrics::default());
        let task = tokio::spawn(run(cache.clone(), fake.clone(), 3, metrics, token.clone()));

        let deadline = Instant::now() + Duration::from_secs(5);
        while cache.snapshot().records[0].resolved_positive < 20 {
            assert!(Instant::now() < deadline, "not resolved in time");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(fake.most_running.load(Ordering::SeqCst) <= 3);
        assert_eq!(
            cache.reader().lookup(&q("8.8.8.7")),
            Some(Answer::Name("host7.example".into()))
        );

        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("returns on cancel")
            .unwrap();
    }

    #[tokio::test]
    async fn cancel_aborts_lookups_in_flight_and_clears_their_marks() {
        let cache = Arc::new(Dns::new(&DnsConfig::default()));
        let stuck = Arc::new(Fake {
            delay: Duration::from_secs(3600),
            ..Default::default()
        });
        cache.reader().lookup(&q("8.8.8.8"));
        let token = CancellationToken::new();
        let task = tokio::spawn(run(
            cache.clone(),
            stuck.clone(),
            4,
            Arc::new(Metrics::default()),
            token.clone(),
        ));
        while stuck.calls("8.8.8.8") == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("returns on cancel")
            .unwrap();
        // the aborted lookup released the question: it is queued again
        cache.reader().lookup(&q("8.8.8.8"));
        assert_eq!(cache.requests().len(), 1);
    }

    /// Needs outbound HTTPS to 1.1.1.1: `cargo test -p patrolli-dns -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn a_public_doh_forwarder_answers() {
        let config = config(
            "mode: forward\nforwarders: [https://1.1.1.1/dns-query, https://dns.google/dns-query]",
        );
        let cache = Arc::new(Dns::new(&config));
        let token = CancellationToken::new();
        let task = spawn(
            cache.clone(),
            &config,
            Duration::from_secs(60),
            token.clone(),
        )
        .unwrap();
        cache.reader().lookup(&q("1.1.1.1"));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(Answer::Name(name)) = cache.reader().lookup(&q("1.1.1.1")) {
                assert_eq!(name, "one.one.one.one");
                break;
            }
            assert!(Instant::now() < deadline, "no answer over DoH");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        token.cancel();
        task.await.unwrap();
    }

    /// Needs outbound UDP and TCP 53 to the internet (`dig +trace -x 8.8.8.8`):
    /// `cargo test -p patrolli-dns -- --ignored`. The IPv6 name has 34 labels,
    /// past the recursor's default limit of 24.
    #[tokio::test]
    #[ignore]
    async fn the_recursor_resolves_v4_and_v6_from_the_roots() {
        let config = config("{}");
        let cache = Arc::new(Dns::new(&config));
        let token = CancellationToken::new();
        let task = spawn(
            cache.clone(),
            &config,
            Duration::from_secs(60),
            token.clone(),
        )
        .unwrap();
        for (ip, want) in [
            ("8.8.8.8", "dns.google"),
            ("2001:4860:4860::8888", "dns.google"),
        ] {
            cache.reader().lookup(&q(ip));
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(Answer::Name(name)) = cache.reader().lookup(&q(ip)) {
                    assert_eq!(name, want, "{ip}");
                    break;
                }
                assert!(Instant::now() < deadline, "{ip}: no answer from the roots");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        token.cancel();
        task.await.unwrap();
    }

    fn config(yaml: &str) -> DnsConfig {
        let mut config: DnsConfig = serde_yaml_ng::from_str(yaml).unwrap();
        config.validate().unwrap();
        config
    }
}
