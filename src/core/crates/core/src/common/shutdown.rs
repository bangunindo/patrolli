// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Process shutdown: the token every long-lived task watches, and the one
//! deadline every teardown step takes its time from.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::error;

/// When a teardown has to be done. A step waits at most its own cap or what
/// is left until the deadline, whichever is shorter, so steps that each run
/// to their cap still end together by the deadline.
#[derive(Clone, Copy, Debug)]
pub struct Deadline(Option<Instant>);

impl Deadline {
    /// No deadline: every step waits its own cap. For a teardown that is not
    /// a shutdown, such as a config change closing one sink.
    pub const NONE: Self = Self(None);

    pub fn at(instant: Instant) -> Self {
        Self(Some(instant))
    }

    /// Time left until the deadline, [`Duration::MAX`] without one.
    pub fn remaining(&self) -> Duration {
        match self.0 {
            None => Duration::MAX,
            Some(at) => at.saturating_duration_since(Instant::now()),
        }
    }

    /// `cap`, or the time left until the deadline when that is shorter.
    pub fn budget(&self, cap: Duration) -> Duration {
        cap.min(self.remaining())
    }
}

/// The process-wide shutdown token, and the deadline that starts when it is
/// cancelled.
#[derive(Clone)]
pub struct Shutdown {
    token: CancellationToken,
    timeout: Duration,
    began: Arc<OnceLock<Instant>>,
}

impl Shutdown {
    pub fn new(timeout: Duration) -> Self {
        Self {
            token: CancellationToken::new(),
            timeout,
            began: Arc::default(),
        }
    }

    /// Cancelled when shutdown begins: on a signal, or by a task that has to
    /// bring the process down.
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// `timeout` after shutdown began, [`Deadline::NONE`] before it. Whoever
    /// asks first after the token is cancelled fixes when that was; the
    /// signal task and the teardown both ask as soon as they see it.
    pub fn deadline(&self) -> Deadline {
        if !self.token.is_cancelled() {
            return Deadline::NONE;
        }
        Deadline::at(*self.began.get_or_init(Instant::now) + self.timeout)
    }
}

/// Awaits the task of one teardown step. One that panicked is logged and the
/// teardown goes on: the steps after it still have to run (sinks still have to
/// deliver what they hold), and the panic itself was reported by the hook in
/// [`crate::common::panic`].
pub async fn join_step<T>(step: &'static str, handle: JoinHandle<T>) -> Option<T> {
    match handle.await {
        Ok(value) => Some(value),
        Err(e) => {
            error!(error = %e, step, "shutdown step failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_is_the_cap_without_a_deadline() {
        let cap = Duration::from_secs(10);
        assert_eq!(Deadline::NONE.budget(cap), cap);
        assert_eq!(Deadline::NONE.remaining(), Duration::MAX);
    }

    #[test]
    fn budget_is_cut_to_what_is_left() {
        let deadline = Deadline::at(Instant::now() + Duration::from_secs(2));
        let budget = deadline.budget(Duration::from_secs(10));
        assert!(budget <= Duration::from_secs(2), "{budget:?}");
        assert!(budget > Duration::from_secs(1), "{budget:?}");
        assert_eq!(
            deadline.budget(Duration::from_millis(5)),
            Duration::from_millis(5)
        );
    }

    #[test]
    fn a_passed_deadline_leaves_nothing() {
        let deadline = Deadline::at(Instant::now() - Duration::from_secs(1));
        assert_eq!(deadline.budget(Duration::from_secs(10)), Duration::ZERO);
    }

    #[test]
    fn the_deadline_starts_when_the_token_is_cancelled() {
        let shutdown = Shutdown::new(Duration::from_secs(30));
        assert_eq!(shutdown.deadline().remaining(), Duration::MAX);
        shutdown.token().cancel();
        let first = shutdown.deadline().remaining();
        assert!(first <= Duration::from_secs(30) && first > Duration::from_secs(29));
        // Fixed by the first call, not moved by later ones.
        std::thread::sleep(Duration::from_millis(20));
        assert!(shutdown.deadline().remaining() < first);
    }
}
