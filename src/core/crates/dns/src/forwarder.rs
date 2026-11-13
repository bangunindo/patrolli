// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Upstream resolvers: one [`Forwarder`] per URL, shared by every list that
//! names it, and a [`ForwarderList`] per list (the main `forwarders` and each
//! zone's) that picks among them.
//!
//! Selection, per lookup:
//!
//! - **Rotation**: each lookup starts at the next forwarder of the list, so
//!   first attempts spread evenly.
//! - **Failover**: no usable reply ([`Reply::Unreachable`]: a timeout, a
//!   connection error, a refusal) moves on to the following ones. Anything
//!   else ends the lookup: an answer, positive or negative, and a server
//!   failure ([`Reply::Unresolved`]), which is the forwarder saying the name
//!   cannot be resolved, not that it is down.
//! - **Cooldown**: `forwarder_failure_threshold` consecutive unreachable
//!   replies take a forwarder out of rotation and failover for
//!   `forwarder_cooldown`. A server failure counts as a reply: the fallback of
//!   a recursor is asked mostly about broken zones, and would otherwise be
//!   cooling down all the time. After that it is eligible again; a success resets
//!   it, a failure re-arms the cooldown at once. With every forwarder of a list
//!   cooling down, one (the next in rotation) is still probed, once: a blip that
//!   tripped them all recovers on the first good answer, and an outage costs
//!   one timeout per lookup, not one per forwarder.
//!
//! A forwarder's health is its own: shared between lists, so a server named by
//! two of them is not counted, or cooled down, twice.

use crate::{Resolve, outcome};
use anyhow::{Context, Result};
use arc_swap::ArcSwapOption;
use futures::future::BoxFuture;
use hickory_resolver::config::{
    ConnectionConfig, NameServerConfig, ResolveHosts, ResolverConfig, ResolverOpts,
};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::{Resolver, TokioResolver};
use patrolli::cache::dns::{Outcome, Question, now_ms};
use patrolli::common::backoff::RestartBackoff;
use patrolli::common::internal_metrics::DnsForwarder;
use patrolli::config::dns::{
    Dns as DnsConfig, Forwarder as ForwarderUrl, ForwarderHost, Transport,
};
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// What a forwarder made of a question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// [`Outcome::Found`] or [`Outcome::Negative`].
    Answered(Outcome),
    /// It replied with a server failure: it is up, and could not resolve the
    /// name (a zone whose servers time out or refuse).
    Unresolved,
    /// No usable reply: a timeout, a connection error, or any other error
    /// response (a refusal is about the client, not the name).
    Unreachable,
}

impl Reply {
    fn into_outcome(self) -> Outcome {
        match self {
            Reply::Answered(outcome) => outcome,
            Reply::Unresolved | Reply::Unreachable => Outcome::Failed,
        }
    }
}

/// How one forwarder is asked.
pub trait Upstream: Send + Sync + 'static {
    /// Whether it can be asked at all yet: an `https://` host name resolves
    /// in the background, and until it has the forwarder is left out.
    fn ready(&self) -> bool {
        true
    }

    /// The caller bounds the time.
    fn query<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Reply>;
}

/// The shared limits of every forwarder.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_inflight: usize,
    pub timeout: Duration,
    pub failure_threshold: u32,
    pub cooldown: Duration,
}

impl Limits {
    pub fn of(config: &DnsConfig) -> Self {
        Self {
            max_inflight: config.forwarder_max_inflight,
            timeout: config.forwarder_timeout,
            failure_threshold: config.forwarder_failure_threshold,
            cooldown: config.forwarder_cooldown,
        }
    }
}

/// One upstream resolver and its health.
pub struct Forwarder {
    name: String,
    upstream: Box<dyn Upstream>,
    /// Hickory fails a DoH or TCP request past its `max_active_requests`
    /// instead of queueing it, so this keeps below that.
    inflight: Semaphore,
    limits: Limits,
    consecutive_failures: AtomicU32,
    /// [`now_ms`] before which it is skipped
    skip_until: AtomicU64,
    requests: AtomicU64,
    failures: AtomicU64,
    cooldowns: AtomicU64,
}

impl Forwarder {
    pub fn new(name: impl Into<String>, upstream: Box<dyn Upstream>, limits: Limits) -> Self {
        Self {
            name: name.into(),
            upstream,
            inflight: Semaphore::new(limits.max_inflight),
            limits,
            consecutive_failures: AtomicU32::new(0),
            skip_until: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            cooldowns: AtomicU64::new(0),
        }
    }

    fn eligible(&self, now: u64) -> bool {
        self.upstream.ready() && self.skip_until.load(Ordering::Relaxed) <= now
    }

    async fn ask(&self, question: &Question) -> Reply {
        let Ok(_permit) = self.inflight.acquire().await else {
            return Reply::Unreachable;
        };
        self.requests.fetch_add(1, Ordering::Relaxed);
        let reply = tokio::time::timeout(self.limits.timeout, self.upstream.query(question))
            .await
            .unwrap_or_else(|_| {
                debug!(forwarder = %self.name, "forwarder query timed out");
                Reply::Unreachable
            });
        match reply {
            Reply::Unreachable => self.failed(),
            Reply::Answered(_) | Reply::Unresolved => self.answered(),
        }
        reply
    }

    fn failed(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        let threshold = self.limits.failure_threshold;
        // stays at the threshold, so one failure after a cooldown re-arms it
        let previous = self
            .consecutive_failures
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_add(1).min(threshold))
            })
            .unwrap_or(threshold);
        if previous.saturating_add(1) < threshold {
            return;
        }
        let now = now_ms();
        let until = now.saturating_add(self.limits.cooldown.as_millis() as u64);
        // Lookups that were in flight when it tripped fail together; they
        // extend the one cooldown rather than each counting a new one.
        if self.skip_until.swap(until, Ordering::Relaxed) <= now {
            self.cooldowns.fetch_add(1, Ordering::Relaxed);
        }
        if previous < threshold {
            warn!(
                forwarder = %self.name,
                failures = threshold,
                cooldown_ms = self.limits.cooldown.as_millis() as u64,
                "forwarder entered cooldown"
            );
        }
    }

    fn answered(&self) {
        if self.consecutive_failures.swap(0, Ordering::Relaxed) >= self.limits.failure_threshold {
            self.skip_until.store(0, Ordering::Relaxed);
            info!(forwarder = %self.name, "forwarder recovered");
        }
    }

    pub fn snapshot(&self) -> DnsForwarder {
        DnsForwarder {
            forwarder: self.name.clone(),
            requests: self.requests.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            cooldowns: self.cooldowns.load(Ordering::Relaxed),
            cooling_down: self.consecutive_failures.load(Ordering::Relaxed)
                >= self.limits.failure_threshold,
        }
    }
}

/// One configured list of forwarders, asked in rotation.
pub struct ForwarderList {
    forwarders: Vec<Arc<Forwarder>>,
    next: AtomicUsize,
}

impl ForwarderList {
    pub fn new(forwarders: Vec<Arc<Forwarder>>) -> Self {
        Self {
            forwarders,
            next: AtomicUsize::new(0),
        }
    }
}

impl Resolve for ForwarderList {
    fn resolve<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Outcome> {
        Box::pin(async move {
            let n = self.forwarders.len();
            if n == 0 {
                return Outcome::Failed;
            }
            let start = self.next.fetch_add(1, Ordering::Relaxed);
            let in_turn = || (0..n).map(|i| &self.forwarders[(start + i) % n]);
            let mut asked = false;
            for forwarder in in_turn() {
                if !forwarder.eligible(now_ms()) {
                    continue;
                }
                asked = true;
                match forwarder.ask(question).await {
                    Reply::Unreachable => continue,
                    reply => return reply.into_outcome(),
                }
            }
            if !asked {
                // all cooling down: probe the next ready one, once
                if let Some(forwarder) = in_turn().find(|f| f.upstream.ready()) {
                    return forwarder.ask(question).await.into_outcome();
                }
            }
            Outcome::Failed
        })
    }
}

/// A forwarder asked through a hickory stub resolver. Built at once for an IP
/// host; for an `https://` host name, by [`resolve_host`] once the system
/// resolver has answered.
pub(crate) struct HickoryUpstream {
    name: String,
    resolver: ArcSwapOption<TokioResolver>,
}

impl Upstream for HickoryUpstream {
    fn ready(&self) -> bool {
        self.resolver.load().is_some()
    }

    fn query<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Reply> {
        Box::pin(async move {
            let Some(resolver) = self.resolver.load_full() else {
                return Reply::Unreachable;
            };
            let (name, record_type) = outcome::query(question);
            match resolver.lookup(name, record_type).await {
                Ok(lookup) => Reply::Answered(outcome::from_records(
                    question,
                    lookup.answers(),
                    lookup.authorities(),
                )),
                Err(error) => match outcome::from_net_error(&error) {
                    Some(negative) => Reply::Answered(negative),
                    None if outcome::is_server_failure(&error) => {
                        debug!(forwarder = %self.name, "forwarder could not resolve");
                        Reply::Unresolved
                    }
                    None => {
                        debug!(forwarder = %self.name, error = %error, "forwarder query failed");
                        Reply::Unreachable
                    }
                },
            }
        })
    }
}

impl HickoryUpstream {
    /// The upstream for `url`, ready unless its host is a name, plus that
    /// name when it still has to be resolved.
    pub(crate) fn new(url: &ForwarderUrl, limits: &Limits) -> Result<(Arc<Self>, Option<String>)> {
        let upstream = Arc::new(Self {
            name: url.to_string(),
            resolver: ArcSwapOption::empty(),
        });
        match &url.host {
            ForwarderHost::Ip(ip) => {
                upstream
                    .resolver
                    .store(Some(Arc::new(build_resolver(*ip, url, limits)?)));
                Ok((upstream, None))
            }
            ForwarderHost::Name(name) => Ok((upstream, Some(name.clone()))),
        }
    }
}

/// `Box<dyn Upstream>` over a shared [`HickoryUpstream`], so the host-name
/// task can fill it in after the [`Forwarder`] owns it.
pub(crate) struct Shared(pub(crate) Arc<HickoryUpstream>);

impl Upstream for Shared {
    fn ready(&self) -> bool {
        self.0.ready()
    }

    fn query<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Reply> {
        self.0.query(question)
    }
}

/// Resolve an `https://` forwarder's host name with the system resolver (not
/// through this one: no bootstrap loop), retrying with backoff until it
/// resolves or `token` is cancelled. One `warn` when it first fails, one
/// `info` when it resolves after that.
pub(crate) async fn resolve_host(
    upstream: Arc<HickoryUpstream>,
    url: ForwarderUrl,
    host: String,
    limits: Limits,
    token: CancellationToken,
) {
    let mut backoff = RestartBackoff::default();
    let mut failing = false;
    loop {
        let resolved = tokio::net::lookup_host((host.as_str(), url.port))
            .await
            .context("resolve the forwarder host")
            .and_then(|mut addrs| {
                addrs
                    .next()
                    .map(|addr| addr.ip())
                    .context("the forwarder host has no address")
            })
            .and_then(|ip| build_resolver(ip, &url, &limits));
        match resolved {
            Ok(resolver) => {
                upstream.resolver.store(Some(Arc::new(resolver)));
                if failing {
                    info!(forwarder = %url, "forwarder host resolved");
                }
                return;
            }
            Err(e) => {
                if !failing {
                    warn!(
                        error = %format!("{e:#}"),
                        forwarder = %url,
                        "forwarder host did not resolve, retrying"
                    );
                    failing = true;
                }
                let delay = backoff.on_exit(tokio::time::Instant::now());
                tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
}

fn build_resolver(ip: IpAddr, url: &ForwarderUrl, limits: &Limits) -> Result<TokioResolver> {
    let mut connection = match url.transport {
        Transport::Udp => ConnectionConfig::udp(),
        Transport::Tcp => ConnectionConfig::tcp(),
        Transport::Https => ConnectionConfig::https(
            url.host.to_string().trim_matches(['[', ']']).into(),
            url.path.as_deref().map(Arc::from),
        ),
    };
    connection.port = url.port;
    let name_server = NameServerConfig::new(ip, true, vec![connection]);

    let mut options = ResolverOpts::default();
    options.timeout = limits.timeout;
    options.attempts = 1;
    options.ndots = 0;
    options.use_hosts_file = ResolveHosts::Never;
    options.num_concurrent_reqs = 1;
    // TCP and DoH refuse requests past this instead of queueing them; the
    // forwarder's semaphore keeps below it.
    options.max_active_requests = limits.max_inflight;
    // `cache::dns` caches every answer; hickory's own cache would only hold
    // a second copy.
    options.cache_size = 1;

    Resolver::builder_with_config(
        ResolverConfig::from_name_servers(vec![name_server]),
        TokioRuntimeProvider::default(),
    )
    .with_options(options)
    .build()
    .with_context(|| format!("build the resolver for {url}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use fake::{Fake, Script};

    /// A forwarder that answers what its script says.
    pub(crate) mod fake {
        use super::*;
        use std::sync::Mutex;

        #[derive(Default)]
        pub(crate) struct Script {
            pub(crate) reply: Mutex<Option<Reply>>,
            pub(crate) calls: AtomicU64,
            pub(crate) delay: Duration,
        }

        impl Script {
            pub(crate) fn answering(outcome: Option<Outcome>) -> Arc<Self> {
                Self::replying(outcome.map(Reply::Answered))
            }

            pub(crate) fn replying(reply: Option<Reply>) -> Arc<Self> {
                Arc::new(Self {
                    reply: Mutex::new(reply),
                    ..Default::default()
                })
            }

            pub(crate) fn set(&self, outcome: Option<Outcome>) {
                *self.reply.lock().unwrap() = outcome.map(Reply::Answered);
            }

            pub(crate) fn calls(&self) -> u64 {
                self.calls.load(Ordering::Relaxed)
            }
        }

        /// `None` is unreachable; a delay past the forwarder timeout times out.
        pub(crate) struct Fake(pub(crate) Arc<Script>);

        impl Upstream for Fake {
            fn query<'a>(&'a self, _: &'a Question) -> BoxFuture<'a, Reply> {
                Box::pin(async move {
                    self.0.calls.fetch_add(1, Ordering::Relaxed);
                    tokio::time::sleep(self.0.delay).await;
                    self.0
                        .reply
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or(Reply::Unreachable)
                })
            }
        }
    }

    fn found() -> Option<Outcome> {
        Some(Outcome::Found {
            answer: patrolli::cache::dns::Answer::Name("a.example".into()),
            ttl: Duration::from_secs(60),
        })
    }

    fn limits(cooldown_ms: u64) -> Limits {
        Limits {
            max_inflight: 4,
            timeout: Duration::from_millis(200),
            failure_threshold: 2,
            cooldown: Duration::from_millis(cooldown_ms),
        }
    }

    fn make_list(scripts: &[Arc<Script>], limits: Limits) -> (ForwarderList, Vec<Arc<Forwarder>>) {
        let forwarders: Vec<_> = scripts
            .iter()
            .enumerate()
            .map(|(i, script)| {
                Arc::new(Forwarder::new(
                    format!("udp://192.0.2.{i}:53"),
                    Box::new(Fake(script.clone())),
                    limits,
                ))
            })
            .collect();
        (ForwarderList::new(forwarders.clone()), forwarders)
    }

    fn q() -> Question {
        Question::Ptr("8.8.8.8".parse().unwrap())
    }

    #[tokio::test]
    async fn first_attempts_rotate() {
        let scripts: Vec<_> = (0..3).map(|_| Script::answering(found())).collect();
        let (list, _) = make_list(&scripts, limits(1000));
        for _ in 0..6 {
            assert!(matches!(list.resolve(&q()).await, Outcome::Found { .. }));
        }
        assert!(scripts.iter().all(|s| s.calls() == 2), "spread evenly");
    }

    #[tokio::test]
    async fn a_failure_fails_over_and_a_negative_answer_does_not() {
        let failing = Script::answering(None);
        let negative = Script::answering(Some(Outcome::Negative { ttl: None }));
        let spare = Script::answering(found());
        let (list, _) = make_list(
            &[failing.clone(), negative.clone(), spare.clone()],
            limits(1000),
        );
        assert_eq!(list.resolve(&q()).await, Outcome::Negative { ttl: None });
        assert_eq!(
            (failing.calls(), negative.calls(), spare.calls()),
            (1, 1, 0)
        );

        // a timeout fails over too
        let slow = Arc::new(Script {
            delay: Duration::from_secs(5),
            ..Default::default()
        });
        let (list, forwarders) = make_list(&[slow.clone(), spare.clone()], limits(1000));
        assert!(matches!(list.resolve(&q()).await, Outcome::Found { .. }));
        assert_eq!(forwarders[0].snapshot().failures, 1);
    }

    #[tokio::test]
    async fn a_forwarder_cools_down_after_the_threshold_and_returns_after_it() {
        let flaky = Script::answering(None);
        let steady = Script::answering(found());
        let (list, forwarders) = make_list(&[flaky.clone(), steady.clone()], limits(300));
        // the threshold is 2: every other lookup starts at the flaky one
        for _ in 0..4 {
            list.resolve(&q()).await;
        }
        assert_eq!(flaky.calls(), 2);
        let snapshot = forwarders[0].snapshot();
        assert!(snapshot.cooling_down);
        assert_eq!((snapshot.failures, snapshot.cooldowns), (2, 1));

        // skipped while cooling down, whatever the rotation
        for _ in 0..4 {
            list.resolve(&q()).await;
        }
        assert_eq!(flaky.calls(), 2);

        // after the cooldown it is asked again; one more failure re-arms it
        tokio::time::sleep(Duration::from_millis(350)).await;
        for _ in 0..2 {
            list.resolve(&q()).await;
        }
        assert_eq!(flaky.calls(), 3);
        assert_eq!(forwarders[0].snapshot().cooldowns, 2);
        for _ in 0..4 {
            list.resolve(&q()).await;
        }
        assert_eq!(flaky.calls(), 3, "re-armed at once");

        // a success after the next cooldown resets it
        flaky.set(found());
        tokio::time::sleep(Duration::from_millis(350)).await;
        for _ in 0..2 {
            list.resolve(&q()).await;
        }
        assert_eq!(flaky.calls(), 4);
        assert!(!forwarders[0].snapshot().cooling_down);
    }

    #[tokio::test]
    async fn with_all_cooling_down_exactly_one_is_probed() {
        let scripts: Vec<_> = (0..3).map(|_| Script::answering(None)).collect();
        let (list, forwarders) = make_list(&scripts, limits(60_000));
        // trip them all: two failing lookups ask every forwarder twice
        for _ in 0..2 {
            assert_eq!(list.resolve(&q()).await, Outcome::Failed);
        }
        assert!(forwarders.iter().all(|f| f.snapshot().cooling_down));
        let before: u64 = scripts.iter().map(|s| s.calls()).sum();
        assert_eq!(list.resolve(&q()).await, Outcome::Failed);
        let after: u64 = scripts.iter().map(|s| s.calls()).sum();
        assert_eq!(after - before, 1);

        // and a good answer from the probe recovers that one
        for s in &scripts {
            s.set(found());
        }
        assert!(matches!(list.resolve(&q()).await, Outcome::Found { .. }));
        assert_eq!(
            forwarders
                .iter()
                .filter(|f| !f.snapshot().cooling_down)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn a_server_failure_ends_the_lookup_and_is_not_held_against_the_forwarder() {
        let unresolved = Script::replying(Some(Reply::Unresolved));
        let spare = Script::answering(found());
        let (list, forwarders) = make_list(&[unresolved.clone(), spare.clone()], limits(60_000));
        // well past the threshold of 2, each lookup starting at the first
        for _ in 0..5 {
            list.next.store(0, Ordering::Relaxed);
            assert_eq!(list.resolve(&q()).await, Outcome::Failed);
        }
        assert_eq!((unresolved.calls(), spare.calls()), (5, 0));
        let snapshot = forwarders[0].snapshot();
        assert!(!snapshot.cooling_down);
        assert_eq!((snapshot.failures, snapshot.cooldowns), (0, 0));

        // and it resets a count of real failures, like any reply
        *unresolved.reply.lock().unwrap() = None;
        list.next.store(0, Ordering::Relaxed);
        list.resolve(&q()).await;
        *unresolved.reply.lock().unwrap() = Some(Reply::Unresolved);
        list.next.store(0, Ordering::Relaxed);
        list.resolve(&q()).await;
        *unresolved.reply.lock().unwrap() = None;
        list.next.store(0, Ordering::Relaxed);
        list.resolve(&q()).await;
        assert!(!forwarders[0].snapshot().cooling_down);
    }

    /// Through hickory, against a UDP server answering with one response code.
    #[tokio::test]
    async fn response_codes_are_told_apart() {
        use hickory_resolver::proto::op::{Message, MessageType, OpCode, ResponseCode};

        async fn reply_of(code: ResponseCode) -> Reply {
            let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let port = socket.local_addr().unwrap().port();
            tokio::spawn(async move {
                let mut buf = vec![0; 4096];
                while let Ok((len, peer)) = socket.recv_from(&mut buf).await {
                    let request = Message::from_vec(&buf[..len]).unwrap();
                    let mut response =
                        Message::new(request.metadata.id, MessageType::Response, OpCode::Query);
                    response.metadata.recursion_desired = true;
                    response.metadata.recursion_available = true;
                    response.metadata.response_code = code;
                    for query in &request.queries {
                        response.add_query(query.clone());
                    }
                    let _ = socket.send_to(&response.to_vec().unwrap(), peer).await;
                }
            });
            let url = format!("udp://127.0.0.1:{port}").parse().unwrap();
            let (upstream, _) = HickoryUpstream::new(&url, &limits(1000)).unwrap();
            upstream.query(&q()).await
        }

        assert_eq!(reply_of(ResponseCode::ServFail).await, Reply::Unresolved);
        assert_eq!(reply_of(ResponseCode::Refused).await, Reply::Unreachable);
        assert_eq!(
            reply_of(ResponseCode::NXDomain).await,
            Reply::Answered(Outcome::Negative { ttl: None })
        );
    }

    #[tokio::test]
    async fn a_forwarder_not_ready_is_left_out() {
        struct NotReady;
        impl Upstream for NotReady {
            fn ready(&self) -> bool {
                false
            }
            fn query<'a>(&'a self, _: &'a Question) -> BoxFuture<'a, Reply> {
                unreachable!("never asked")
            }
        }
        let list = ForwarderList::new(vec![Arc::new(Forwarder::new(
            "https://dns.example:443/dns-query",
            Box::new(NotReady),
            limits(1000),
        ))]);
        assert_eq!(list.resolve(&q()).await, Outcome::Failed);
    }
}
