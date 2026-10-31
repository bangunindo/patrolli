// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Circuit breaker for one endpoint of a sink with several.
//!
//! A failed send marks the endpoint unhealthy for a while that grows with
//! each consecutive failure. Once that has passed, a single send is let
//! through as a probe (half-open); its outcome decides between healthy and
//! another, longer wait.

use super::uptime;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use tracing::{info, warn};

const RETRY_INTERVAL: Duration = Duration::from_secs(5);
const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(60);

pub struct Health {
    /// Only for logging transitions and reporting.
    endpoint: String,
    healthy: AtomicBool,
    /// Millis on the [`uptime`] clock.
    retry_at_ms: AtomicU64,
    consecutive_failures: AtomicU32,
    /// The half-open slot.
    probing: AtomicBool,
}

impl Health {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            healthy: AtomicBool::new(true),
            retry_at_ms: AtomicU64::new(uptime().as_millis() as u64),
            consecutive_failures: AtomicU32::new(0),
            probing: AtomicBool::new(false),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    pub fn set_healthy(&self) {
        if !self.is_healthy() {
            let failures = self.consecutive_failures.load(Ordering::Relaxed);
            self.healthy.store(true, Ordering::Relaxed);
            self.consecutive_failures.store(0, Ordering::Relaxed);
            info!(endpoint = %self.endpoint, failures, "endpoint recovered");
        }
    }

    pub fn set_unhealthy(&self) {
        let was_healthy = self.healthy.swap(false, Ordering::Relaxed);
        let failures = if was_healthy {
            self.consecutive_failures.store(1, Ordering::Relaxed);
            1
        } else {
            self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1
        };
        let backoff = (RETRY_INTERVAL * failures).min(MAX_RETRY_INTERVAL);
        self.retry_at_ms
            .store((uptime() + backoff).as_millis() as u64, Ordering::Relaxed);
        if was_healthy {
            warn!(endpoint = %self.endpoint, retry_in = ?backoff, "endpoint marked unhealthy");
        }
    }

    /// A slot to send through: always while healthy, else the one probe once
    /// the wait has passed. The guard settles the health when dropped, with
    /// the outcome it was given.
    pub fn try_send(&self) -> Option<SendGuard<'_>> {
        if self.is_healthy() {
            return Some(SendGuard {
                health: self,
                probe: false,
                ok: None,
            });
        }
        let due = self.retry_at_ms.load(Ordering::Relaxed) < uptime().as_millis() as u64;
        if due
            && self
                .probing
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        {
            return Some(SendGuard {
                health: self,
                probe: true,
                ok: None,
            });
        }
        None
    }

    /// How long until [`try_send`](Self::try_send) can hand out a slot: zero
    /// while healthy or once the wait has passed (a probe may still be out).
    pub fn retry_in(&self) -> Duration {
        if self.is_healthy() {
            return Duration::ZERO;
        }
        let at = self.retry_at_ms.load(Ordering::Relaxed);
        Duration::from_millis(at.saturating_sub(uptime().as_millis() as u64))
    }
}

/// A send through a [`Health`]. Dropped without an outcome (the send was
/// cancelled), it only gives the probe slot back: an endpoint must not go
/// down because a worker stopped.
pub struct SendGuard<'a> {
    health: &'a Health,
    probe: bool,
    ok: Option<bool>,
}

impl SendGuard<'_> {
    pub fn succeeded(&mut self) {
        self.ok = Some(true);
    }

    pub fn failed(&mut self) {
        self.ok = Some(false);
    }
}

impl Drop for SendGuard<'_> {
    fn drop(&mut self) {
        match self.ok {
            Some(true) => self.health.set_healthy(),
            Some(false) => self.health.set_unhealthy(),
            None => {}
        }
        if self.probe {
            self.health.probing.store(false, Ordering::Release); // last
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_failure_closes_the_endpoint_until_one_probe_is_due() {
        let health = Health::new("a");
        health.try_send().unwrap().failed();
        assert!(!health.is_healthy());
        assert!(health.try_send().is_none());
        assert_eq!(health.retry_in(), RETRY_INTERVAL);

        tokio::time::advance(RETRY_INTERVAL + Duration::from_millis(1)).await;
        assert_eq!(health.retry_in(), Duration::ZERO);
        let mut probe = health.try_send().expect("a probe once due");
        assert!(health.try_send().is_none(), "only one probe at a time");
        probe.succeeded();
        drop(probe);
        assert!(health.is_healthy());
        assert!(health.try_send().is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_grows_with_each_failed_probe() {
        let health = Health::new("a");
        health.try_send().unwrap().failed();
        tokio::time::advance(RETRY_INTERVAL + Duration::from_millis(1)).await;
        health.try_send().unwrap().failed();
        assert_eq!(health.retry_in(), RETRY_INTERVAL * 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_send_dropped_without_an_outcome_changes_nothing_but_frees_the_probe() {
        let health = Health::new("a");
        drop(health.try_send());
        assert!(health.is_healthy());

        health.try_send().unwrap().failed();
        tokio::time::advance(RETRY_INTERVAL + Duration::from_millis(1)).await;
        drop(health.try_send().expect("a probe once due"));
        assert!(!health.is_healthy());
        assert!(health.try_send().is_some(), "the probe slot is free again");
    }
}
