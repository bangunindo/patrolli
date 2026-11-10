// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Restart pacing for supervised workers, and retry pacing for sinks.

use crate::config;
use tokio::time::{Duration, Instant};

const MIN_DELAY: Duration = Duration::from_secs(1);
const MAX_DELAY: Duration = Duration::from_secs(60);
/// An exit this long after the previous one starts the sequence over, so a
/// worker that dies once a day always comes back after `MIN_DELAY`.
const RESET_AFTER: Duration = Duration::from_secs(120);

/// Exponential delay between restarts of a worker that keeps exiting:
/// 1s, 2s, 4s, … capped at 60s.
#[derive(Debug, Default)]
pub struct RestartBackoff {
    attempts: u32,
    last_exit: Option<Instant>,
    next_at: Option<Instant>,
}

impl RestartBackoff {
    /// Records an unexpected exit and returns the delay before the next restart.
    pub fn on_exit(&mut self, now: Instant) -> Duration {
        if self
            .last_exit
            .is_some_and(|last| now.duration_since(last) > RESET_AFTER)
        {
            self.attempts = 0;
        }
        self.attempts = self.attempts.saturating_add(1);
        self.last_exit = Some(now);
        let delay = MIN_DELAY
            .checked_mul(1 << (self.attempts - 1).min(31))
            .map_or(MAX_DELAY, |d| d.min(MAX_DELAY));
        self.next_at = Some(now + delay);
        delay
    }

    /// Consecutive exits in the current sequence.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Whether the delay set by the last [`on_exit`](Self::on_exit) has passed.
    pub fn ready(&self, now: Instant) -> bool {
        self.next_at.is_none_or(|at| now >= at)
    }
}

/// How far [`RetryBackoff`] randomizes each delay, either way: spread over
/// ±20%, workers that failed together do not all retry together.
const RETRY_JITTER: f64 = 0.2;

/// Delay between attempts at a batch the destination did not take:
/// `initial_interval`, growing by `multiplier` up to `max_interval`, each
/// randomized by [`RETRY_JITTER`]. Unlike [`RestartBackoff`] it starts over
/// only when told to, on the next success.
#[derive(Debug, Clone)]
pub struct RetryBackoff {
    initial: Duration,
    max: Duration,
    multiplier: f64,
    jitter: f64,
    current: Duration,
}

impl RetryBackoff {
    /// `conf` must be validated: a multiplier below 1 or a `max_interval`
    /// shorter than `initial_interval` would not grow towards the cap.
    pub fn new(conf: &config::Retry) -> Self {
        Self {
            initial: conf.initial_interval,
            max: conf.max_interval,
            multiplier: conf.multiplier,
            jitter: RETRY_JITTER,
            current: conf.initial_interval,
        }
    }

    /// The same pacing without the randomization, for tests that check delays.
    pub fn without_jitter(mut self) -> Self {
        self.jitter = 0.0;
        self
    }

    /// The delay before the next attempt.
    pub fn next_delay(&mut self) -> Duration {
        let base = self.current;
        self.current = self.current.mul_f64(self.multiplier).min(self.max);
        if self.jitter > 0.0 {
            let factor = rand::random_range(1.0 - self.jitter..=1.0 + self.jitter);
            base.mul_f64(factor).min(self.max)
        } else {
            base
        }
    }

    /// Starts the sequence over, once an attempt got through.
    pub fn reset(&mut self) {
        self.current = self.initial;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retry(initial: u64, max: u64, multiplier: f64) -> RetryBackoff {
        RetryBackoff::new(&config::Retry {
            initial_interval: Duration::from_secs(initial),
            max_interval: Duration::from_secs(max),
            multiplier,
            max_elapsed: Duration::ZERO,
        })
    }

    #[test]
    fn retry_delay_grows_up_to_the_cap_and_resets() {
        let mut backoff = retry(1, 60, 2.0).without_jitter();
        let delays: Vec<u64> = (0..9).map(|_| backoff.next_delay().as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
    }

    #[test]
    fn a_multiplier_of_one_keeps_the_delay_constant() {
        let mut backoff = retry(5, 60, 1.0).without_jitter();
        assert!((0..5).all(|_| backoff.next_delay() == Duration::from_secs(5)));
    }

    #[test]
    fn retry_jitter_stays_within_its_bounds_and_under_the_cap() {
        let mut backoff = retry(10, 15, 2.0);
        let first = backoff.next_delay();
        assert!(first >= Duration::from_secs(8) && first <= Duration::from_secs(12));
        for _ in 0..100 {
            let delay = backoff.next_delay();
            assert!(delay >= Duration::from_secs(12) && delay <= Duration::from_secs(15));
        }
    }

    #[test]
    fn delay_doubles_up_to_the_cap() {
        let mut backoff = RestartBackoff::default();
        let now = Instant::now();
        let delays: Vec<u64> = (0..9).map(|_| backoff.on_exit(now).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(backoff.attempts(), 9);
    }

    #[test]
    fn a_quiet_period_starts_the_sequence_over() {
        let mut backoff = RestartBackoff::default();
        let now = Instant::now();
        backoff.on_exit(now);
        backoff.on_exit(now);
        assert_eq!(backoff.on_exit(now + RESET_AFTER), Duration::from_secs(4));
        let later = now + RESET_AFTER + RESET_AFTER + Duration::from_secs(1);
        assert_eq!(backoff.on_exit(later), MIN_DELAY);
        assert_eq!(backoff.attempts(), 1);
    }

    #[test]
    fn ready_only_once_the_delay_has_passed() {
        let mut backoff = RestartBackoff::default();
        let now = Instant::now();
        assert!(backoff.ready(now));
        let delay = backoff.on_exit(now);
        assert!(!backoff.ready(now));
        assert!(!backoff.ready(now + delay - Duration::from_millis(1)));
        assert!(backoff.ready(now + delay));
    }

    #[test]
    fn attempts_never_overflow() {
        let mut backoff = RestartBackoff {
            attempts: u32::MAX,
            ..Default::default()
        };
        assert_eq!(backoff.on_exit(Instant::now()), MAX_DELAY);
    }
}
