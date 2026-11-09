// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The routers of one listener: which session is live for each, and when the
//! routes a lost session left behind must go.
//!
//! A session that ends (disconnect, Termination, error) leaves its routes in
//! the table for `keep`, so a router that reconnects promptly is never seen
//! with an empty table. A reconnected router's End-of-RIB sweeps what its new
//! dump did not refresh; the `keep` deadline sweeps whatever is left after
//! that: VRFs that never came back, routers that send no End-of-RIB, and
//! routers that stay away.
//!
//! A router has one live session at a time, of either protocol. A new
//! session of the same protocol replaces the live one (the router restarted
//! or reconnected); one of the other protocol is refused while the live one
//! lasts, so two feeds never write the same router's partitions.

use parking_lot::Mutex;
use patrolli::cache::lookup_table::mutable::now_millis;
use rustc_hash::{FxHashMap, FxHashSet};
use smol_str::SmolStr;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// How long a new session waits for the one it replaces to stop writing.
pub const SUPERSEDE_WAIT: Duration = Duration::from_secs(5);

/// How a router feeds the listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    Bmp,
    Bgp,
}

impl Protocol {
    pub const ALL: [Self; 2] = [Self::Bmp, Self::Bgp];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bmp => "bmp",
            Self::Bgp => "bgp",
        }
    }
}

/// A session refused because its router has a live one of the other
/// protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused {
    pub live: Protocol,
}

#[derive(Debug, Default)]
struct Router {
    generation: u64,
    live: Option<Live>,
    /// the protocol of the last session, for what its routes' expiry reports;
    /// none for routes a previous collector left
    protocol: Option<Protocol>,
    /// the VRFs (route table partitions) written for this router
    vrfs: FxHashSet<SmolStr>,
    /// when routes not refreshed by then go
    deadline: Option<Instant>,
}

#[derive(Debug)]
struct Live {
    generation: u64,
    protocol: Protocol,
    /// the live session's start, in `now_millis` time
    cutoff: u64,
    cancel: CancellationToken,
    /// cancelled once the session has stopped writing
    done: CancellationToken,
}

/// A session's registration, from [`Routers::attach`].
#[derive(Debug)]
pub struct Attachment {
    pub router: IpAddr,
    pub protocol: Protocol,
    pub generation: u64,
    /// the session's start: rows older than this were not written by it
    pub cutoff: u64,
    done: CancellationToken,
}

/// Routes to sweep for a router whose deadline passed.
#[derive(Debug, PartialEq, Eq)]
pub struct Expired {
    pub router: IpAddr,
    /// the protocol of the session that left the routes, if known
    pub protocol: Option<Protocol>,
    pub vrfs: Vec<SmolStr>,
    /// rows written before this go: the live session's start, or the time the
    /// deadline was handled when no session is live
    pub cutoff: u64,
}

#[derive(Debug, Default)]
pub struct Routers {
    inner: Mutex<FxHashMap<IpAddr, Router>>,
    /// a deadline was armed
    changed: Notify,
    closing: AtomicBool,
}

impl Routers {
    /// Registers a session for `router`, ending and waiting for (up to
    /// [`SUPERSEDE_WAIT`]) a live one of the same protocol, so two sessions
    /// never write the same router's routes at once. Returns whether a live
    /// session was replaced; a live session of the other protocol refuses
    /// this one.
    pub async fn attach(
        &self,
        router: IpAddr,
        protocol: Protocol,
        cancel: CancellationToken,
        keep: Duration,
    ) -> Result<(Attachment, bool), Refused> {
        let done = CancellationToken::new();
        let (attachment, previous) = {
            let mut routers = self.inner.lock();
            let r = routers.entry(router).or_default();
            if let Some(live) = &r.live
                && live.protocol != protocol
            {
                return Err(Refused {
                    live: live.protocol,
                });
            }
            r.generation += 1;
            r.protocol = Some(protocol);
            // a reconnect gets a whole `keep` to dump and send End-of-RIB,
            // whether the old session had ended or is replaced now; the
            // deadline then sweeps what the new one did not refresh
            if r.deadline.is_some() || r.live.is_some() {
                r.deadline = Some(Instant::now() + keep);
                self.changed.notify_one();
            }
            let cutoff = now_millis();
            let previous = r.live.replace(Live {
                generation: r.generation,
                protocol,
                cutoff,
                cancel,
                done: done.clone(),
            });
            (
                Attachment {
                    router,
                    protocol,
                    generation: r.generation,
                    cutoff,
                    done,
                },
                previous,
            )
        };
        let superseded = previous.is_some();
        if let Some(previous) = previous {
            previous.cancel.cancel();
            // past the wait, the old session is stuck in a write it will
            // finish soon; carrying on is better than refusing the router
            let _ = tokio::time::timeout(SUPERSEDE_WAIT, previous.done.cancelled()).await;
        }
        Ok((attachment, superseded))
    }

    /// Unregisters a session. Unless it was replaced or the listener is
    /// closing, its routes are kept until `keep` from now.
    pub fn detach(&self, attachment: Attachment, keep: Duration) {
        {
            let mut routers = self.inner.lock();
            if let Some(r) = routers.get_mut(&attachment.router)
                && r.live
                    .as_ref()
                    .is_some_and(|l| l.generation == attachment.generation)
            {
                r.live = None;
                if !self.closing.load(Ordering::Relaxed) {
                    r.deadline = Some(Instant::now() + keep);
                    self.changed.notify_one();
                }
            }
        }
        attachment.done.cancel();
    }

    /// Records that `router` has routes in `vrf`.
    pub fn note_vrf(&self, router: IpAddr, vrf: &SmolStr) {
        if let Some(r) = self.inner.lock().get_mut(&router)
            && !r.vrfs.contains(vrf)
        {
            r.vrfs.insert(vrf.clone());
        }
    }

    /// Registers routes a previous collector left in the table for `router`'s
    /// `vrf`: they go at `keep` from now unless a session from the router
    /// refreshes them first, as if the router had just disconnected.
    pub fn seed(&self, router: IpAddr, vrf: SmolStr, keep: Duration) {
        let mut routers = self.inner.lock();
        let r = routers.entry(router).or_default();
        r.vrfs.insert(vrf);
        if r.live.is_none() && r.deadline.is_none() {
            r.deadline = Some(Instant::now() + keep);
            self.changed.notify_one();
        }
    }

    /// How many routers have a live session.
    pub fn live(&self) -> usize {
        self.inner
            .lock()
            .values()
            .filter(|r| r.live.is_some())
            .count()
    }

    /// The earliest armed deadline.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.inner.lock().values().filter_map(|r| r.deadline).min()
    }

    /// Resolves after a deadline is armed. Create it before reading
    /// [`Self::next_deadline`], so an arming in between is not missed.
    pub fn changed(&self) -> tokio::sync::futures::Notified<'_> {
        self.changed.notified()
    }

    /// Takes the routers whose deadline is at or before `now`. A router with
    /// no live session is forgotten: its routes are all about to go.
    pub fn take_expired(&self, now: Instant) -> Vec<Expired> {
        let mut routers = self.inner.lock();
        let cutoff_now = now_millis();
        let mut expired = Vec::new();
        routers.retain(|router, r| {
            if r.deadline.is_none_or(|d| d > now) {
                return true;
            }
            r.deadline = None;
            let cutoff = r.live.as_ref().map_or(cutoff_now, |l| l.cutoff);
            expired.push(Expired {
                router: *router,
                protocol: r.protocol,
                vrfs: r.vrfs.iter().cloned().collect(),
                cutoff,
            });
            r.live.is_some()
        });
        expired
    }

    /// Ends every live session for good: no deadline is armed afterwards.
    pub fn close_all(&self) {
        self.closing.store(true, Ordering::Relaxed);
        for r in self.inner.lock().values() {
            if let Some(live) = &r.live {
                live.cancel.cancel();
            }
        }
    }

    /// Whether a newer session of the attachment's router replaced it.
    pub fn is_replaced(&self, attachment: &Attachment) -> bool {
        self.inner
            .lock()
            .get(&attachment.router)
            .and_then(|r| r.live.as_ref())
            .is_some_and(|l| l.generation != attachment.generation)
    }

    pub fn is_live(&self, router: IpAddr) -> bool {
        self.inner
            .lock()
            .get(&router)
            .is_some_and(|r| r.live.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEEP: Duration = Duration::from_secs(300);

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn a_lost_session_arms_keep_and_expires_its_router() {
        let routers = Routers::default();
        let (a, superseded) = routers
            .attach(
                ip("192.0.2.1"),
                Protocol::Bmp,
                CancellationToken::new(),
                KEEP,
            )
            .await
            .unwrap();
        assert!(!superseded);
        routers.note_vrf(a.router, &"global".into());
        assert_eq!(routers.next_deadline(), None, "a live session has none");

        routers.detach(a, KEEP);
        let deadline = routers.next_deadline().unwrap();
        assert!(
            routers
                .take_expired(deadline - Duration::from_secs(1))
                .is_empty()
        );
        let expired = routers.take_expired(deadline);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].vrfs, ["global"]);
        assert_eq!(routers.next_deadline(), None);
        assert!(routers.take_expired(deadline).is_empty(), "forgotten");
    }

    #[tokio::test(start_paused = true)]
    async fn a_reconnect_extends_the_deadline_and_sweeps_with_its_own_start() {
        let routers = Routers::default();
        let (a, _) = routers
            .attach(
                ip("192.0.2.1"),
                Protocol::Bmp,
                CancellationToken::new(),
                KEEP,
            )
            .await
            .unwrap();
        routers.note_vrf(a.router, &"global".into());
        routers.detach(a, KEEP);
        let first = routers.next_deadline().unwrap();

        tokio::time::advance(Duration::from_secs(200)).await;
        let (b, _) = routers
            .attach(
                ip("192.0.2.1"),
                Protocol::Bmp,
                CancellationToken::new(),
                KEEP,
            )
            .await
            .unwrap();
        let extended = routers.next_deadline().unwrap();
        assert!(extended > first, "the new dump gets a whole keep");

        let expired = routers.take_expired(extended);
        assert_eq!(expired[0].cutoff, b.cutoff);
        assert!(routers.is_live(b.router), "a live router is not forgotten");
        assert_eq!(routers.next_deadline(), None);
        routers.detach(b, KEEP);
        assert!(routers.next_deadline().is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_session_ends_the_old_one_first() {
        let routers = std::sync::Arc::new(Routers::default());
        let old_cancel = CancellationToken::new();
        let (old, _) = routers
            .attach(ip("192.0.2.1"), Protocol::Bmp, old_cancel.clone(), KEEP)
            .await
            .unwrap();

        let r = routers.clone();
        let new = tokio::spawn(async move {
            r.attach(
                ip("192.0.2.1"),
                Protocol::Bmp,
                CancellationToken::new(),
                KEEP,
            )
            .await
            .unwrap()
        });
        old_cancel.cancelled().await;
        assert!(!new.is_finished(), "waits for the old session to stop");
        // the old session ends: replaced, so its detach arms nothing more
        routers.detach(old, KEEP);
        let (new, superseded) = new.await.unwrap();
        assert!(superseded);
        assert!(routers.is_live(new.router));

        // the replacing session got a whole `keep` (armed at attach, before
        // the wait): what it has not refreshed by then goes, with its own
        // start as the cutoff, and the router stays
        let deadline = routers
            .next_deadline()
            .expect("a replacing session arms keep");
        assert_eq!(deadline, Instant::now() + KEEP);
        let expired = routers.take_expired(deadline);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].cutoff, new.cutoff);
        assert!(routers.is_live(new.router));
        assert_eq!(routers.next_deadline(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stuck_session_is_waited_for_only_so_long() {
        let routers = Routers::default();
        let (_old, _) = routers
            .attach(
                ip("192.0.2.1"),
                Protocol::Bmp,
                CancellationToken::new(),
                KEEP,
            )
            .await
            .unwrap();
        let start = Instant::now();
        let (_new, superseded) = routers
            .attach(
                ip("192.0.2.1"),
                Protocol::Bmp,
                CancellationToken::new(),
                KEEP,
            )
            .await
            .unwrap();
        assert!(superseded);
        assert_eq!(start.elapsed(), SUPERSEDE_WAIT);
    }

    #[tokio::test(start_paused = true)]
    async fn seeded_routes_expire_at_keep_unless_the_router_returns() {
        let routers = Routers::default();
        routers.seed(ip("192.0.2.1"), "global".into(), KEEP);
        routers.seed(ip("192.0.2.1"), "65000:1".into(), KEEP);
        routers.seed(ip("192.0.2.2"), "global".into(), KEEP);
        let deadline = routers.next_deadline().unwrap();
        assert_eq!(deadline, Instant::now() + KEEP);

        // 192.0.2.2 comes back: its sweep uses its own session's start
        let (b, _) = routers
            .attach(
                ip("192.0.2.2"),
                Protocol::Bmp,
                CancellationToken::new(),
                KEEP,
            )
            .await
            .unwrap();
        assert_eq!(routers.live(), 1);
        let deadline = routers.next_deadline().unwrap();
        let mut expired = routers.take_expired(deadline);
        expired.sort_by_key(|e| e.router);
        assert_eq!(expired.len(), 2);
        let mut vrfs = expired[0].vrfs.clone();
        vrfs.sort();
        assert_eq!(vrfs, ["65000:1", "global"]);
        assert_eq!(expired[1].cutoff, b.cutoff);
        assert!(!routers.is_live(ip("192.0.2.1")));
        assert!(routers.is_live(ip("192.0.2.2")));
    }

    #[tokio::test(start_paused = true)]
    async fn closing_cancels_sessions_without_arming_deadlines() {
        let routers = Routers::default();
        let cancel = CancellationToken::new();
        let (a, _) = routers
            .attach(ip("192.0.2.1"), Protocol::Bmp, cancel.clone(), KEEP)
            .await
            .unwrap();
        routers.close_all();
        assert!(cancel.is_cancelled());
        routers.detach(a, KEEP);
        assert_eq!(routers.next_deadline(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_router_live_over_one_protocol_refuses_the_other() {
        let routers = Routers::default();
        let cancel = CancellationToken::new();
        let (bmp, _) = routers
            .attach(ip("192.0.2.1"), Protocol::Bmp, cancel.clone(), KEEP)
            .await
            .unwrap();
        let refused = routers
            .attach(
                ip("192.0.2.1"),
                Protocol::Bgp,
                CancellationToken::new(),
                KEEP,
            )
            .await;
        assert_eq!(
            refused.unwrap_err(),
            Refused {
                live: Protocol::Bmp
            }
        );
        assert!(!cancel.is_cancelled(), "the live session goes on");
        assert_eq!(routers.next_deadline(), None);

        // once it is gone, the other protocol takes over and reports as such
        routers.detach(bmp, KEEP);
        let (bgp, superseded) = routers
            .attach(
                ip("192.0.2.1"),
                Protocol::Bgp,
                CancellationToken::new(),
                KEEP,
            )
            .await
            .unwrap();
        assert!(!superseded);
        routers.note_vrf(bgp.router, &"global".into());
        routers.detach(bgp, KEEP);
        let expired = routers.take_expired(routers.next_deadline().unwrap());
        assert_eq!(expired[0].protocol, Some(Protocol::Bgp));
    }
}
