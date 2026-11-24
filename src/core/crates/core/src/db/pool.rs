// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The Postgres pool every backend-mode component shares, counted.
//!
//! deadpool reports its size and queue (`status()`) but neither how long a
//! checkout waited nor how long a connection was held, so [`Pool`] wraps it:
//! [`Pool::get`] times the wait, and the [`Client`] it hands out times the
//! hold until it is dropped.

use crate::common::internal_metrics::{DbPool, INTERNAL_METRICS};
use deadpool_postgres::{BuildError, ClientWrapper, Hook, Object, PoolBuilder, PoolError, Runtime};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Metrics {
    checkouts: AtomicU64,
    checkout_errors: AtomicU64,
    connections_created: AtomicU64,
    wait_nanos: AtomicU64,
    hold_nanos: AtomicU64,
}

#[derive(Clone)]
pub struct Pool {
    inner: deadpool_postgres::Pool,
    metrics: Arc<Metrics>,
}

impl Pool {
    /// Builds the pool from `Config::builder(tls)`, on the tokio runtime.
    pub fn build(builder: PoolBuilder) -> Result<Self, BuildError> {
        let metrics = Arc::new(Metrics::default());
        let created = metrics.clone();
        let inner = builder
            .runtime(Runtime::Tokio1)
            .post_create(Hook::sync_fn(move |_, _| {
                created.connections_created.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }))
            .build()?;
        Ok(Self { inner, metrics })
    }

    /// A connection, back in the pool when the [`Client`] is dropped.
    pub async fn get(&self) -> Result<Client, PoolError> {
        let started = Instant::now();
        match self.inner.get().await {
            Ok(object) => {
                let checked_out = Instant::now();
                self.metrics.checkouts.fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .wait_nanos
                    .fetch_add(nanos(checked_out - started), Ordering::Relaxed);
                Ok(Client {
                    object: Some(object),
                    metrics: self.metrics.clone(),
                    checked_out,
                })
            }
            Err(e) => {
                self.metrics.checkout_errors.fetch_add(1, Ordering::Relaxed);
                Err(e)
            }
        }
    }

    pub fn close(&self) {
        self.inner.close();
    }

    pub fn snapshot(&self) -> DbPool {
        let status = self.inner.status();
        let metrics = &self.metrics;
        DbPool {
            max_size: status.max_size as u64,
            size: status.size as u64,
            available: status.available as u64,
            waiting: status.waiting as u64,
            checkouts: metrics.checkouts.load(Ordering::Relaxed),
            checkout_errors: metrics.checkout_errors.load(Ordering::Relaxed),
            connections_created: metrics.connections_created.load(Ordering::Relaxed),
            checkout_wait_time: Duration::from_nanos(metrics.wait_nanos.load(Ordering::Relaxed)),
            hold_time: Duration::from_nanos(metrics.hold_nanos.load(Ordering::Relaxed)),
        }
    }
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// A checked-out connection. Derefs to deadpool's [`Object`], which is what
/// `&impl GenericClient` wants (`&*client`): the trait is sealed.
pub struct Client {
    /// `None` only once detached or dropped
    object: Option<Object>,
    metrics: Arc<Metrics>,
    checked_out: Instant,
}

impl Client {
    /// Takes the connection out of the pool for good: it closes when dropped
    /// and the pool opens another in its place. Its hold time ends here.
    pub fn detach(mut self) -> ClientWrapper {
        let object = self.object.take().expect("client is attached");
        self.record_hold();
        Object::take(object)
    }

    fn record_hold(&self) {
        self.metrics
            .hold_nanos
            .fetch_add(nanos(self.checked_out.elapsed()), Ordering::Relaxed);
    }
}

impl Deref for Client {
    type Target = Object;

    fn deref(&self) -> &Object {
        self.object.as_ref().expect("client is attached")
    }
}

impl DerefMut for Client {
    fn deref_mut(&mut self) -> &mut Object {
        self.object.as_mut().expect("client is attached")
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.object.is_some() {
            self.record_hold();
        }
    }
}

/// Publishes the pool snapshot every `interval` until `shutdown` fires.
pub fn spawn_metrics_reporter(
    pool: Pool,
    interval: Duration,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(interval) => {
                    INTERNAL_METRICS
                        .db_pool
                        .store(Arc::new((Instant::now(), pool.snapshot())));
                }
            }
        }
    })
}
