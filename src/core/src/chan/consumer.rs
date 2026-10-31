// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::Message;
use super::channel::DiskWriter;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;
use tracing::warn;

/// A payload of a write-through topic, with the disk key that holds it until
/// it is acked.
pub(super) type Leased<M> = (u64, M);

/// Proof that a payload was taken off a topic, handed back through
/// [`Consumer::ack`] once the consumer is done with it (delivered or
/// rejected) or with the payload through [`Consumer::nack`] to requeue it.
///
/// Dropping it does nothing: an aborted worker must not count as done, so a
/// payload it held is only lost where the topic could not have kept it anyway.
/// On a write-through topic its key stays on disk and comes back on the next
/// start.
#[must_use = "hand the token back with Consumer::ack, or the payload with Consumer::nack"]
pub struct AckToken {
    /// The disk key a write-through topic deletes on ack.
    key: Option<u64>,
}

/// A payload taken off a topic, with the token that settles it.
pub struct Delivery<M> {
    pub msg: M,
    token: AckToken,
}

impl<M> Delivery<M> {
    fn new(key: Option<u64>, msg: M) -> Self {
        Self {
            msg,
            token: AckToken { key },
        }
    }

    pub fn into_parts(self) -> (M, AckToken) {
        (self.msg, self.token)
    }
}

/// What a consumer reads: bare payloads, or on a write-through topic payloads
/// with the key they are leased under.
enum Input<M> {
    Plain(flume::Receiver<M>),
    Leased(flume::Receiver<Leased<M>>),
}

impl<M> Clone for Input<M> {
    fn clone(&self) -> Self {
        match self {
            Input::Plain(rx) => Input::Plain(rx.clone()),
            Input::Leased(rx) => Input::Leased(rx.clone()),
        }
    }
}

impl<M> Input<M> {
    async fn recv(&self) -> Option<(Option<u64>, M)> {
        match self {
            Input::Plain(rx) => rx.recv_async().await.ok().map(|msg| (None, msg)),
            Input::Leased(rx) => rx
                .recv_async()
                .await
                .ok()
                .map(|(key, msg)| (Some(key), msg)),
        }
    }
}

enum WeakInput<M> {
    Plain(flume::WeakSender<M>),
    Leased(flume::WeakSender<Leased<M>>),
}

impl<M> WeakInput<M> {
    /// Requeues through the input, or hands the payload back when it has no
    /// room or is gone.
    fn try_send(&self, key: Option<u64>, msg: M) -> Result<(), (Option<u64>, M)> {
        match (self, key) {
            (WeakInput::Plain(weak), None) => {
                let Some(sender) = weak.upgrade() else {
                    return Err((None, msg));
                };
                sender.try_send(msg).map_err(|e| (None, e.into_inner()))
            }
            (WeakInput::Leased(weak), Some(key)) => {
                let Some(sender) = weak.upgrade() else {
                    return Err((Some(key), msg));
                };
                sender.try_send((key, msg)).map_err(|e| {
                    let (key, msg) = e.into_inner();
                    (Some(key), msg)
                })
            }
            // A delivery always comes back to the topic it came from.
            (_, key) => Err((key, msg)),
        }
    }
}

struct Shared<M> {
    topic: String,
    /// The input's own sender, for requeueing. Weak, so the consumer never
    /// keeps its input open: a topic that is closed or removed must still
    /// disconnect, or its workers never see the input end.
    weak: WeakInput<M>,
    /// Set on a disk topic: a requeue goes back to its keyspace, and on a
    /// write-through topic an ack deletes the key.
    disk: Option<DiskWriter>,
    /// Requeued payloads the input had no room for. Bounded by what the
    /// workers have in flight, since a worker only requeues what it took.
    stash: Mutex<VecDeque<(Option<u64>, M)>>,
    /// `stash.len()`, read without the lock on every `recv`.
    stashed: AtomicUsize,
    /// Wakes a worker parked on the input when the stash gains a payload.
    notify: Notify,
}

impl<M> Drop for Shared<M> {
    fn drop(&mut self) {
        let dropped = self.stash.get_mut().len();
        if dropped > 0 {
            warn!(topic = %self.topic, payloads = dropped, "requeued payloads dropped with their consumer");
        }
    }
}

/// The receiving end of a topic, shared by every worker of its sink and
/// handed to the sink that replaces it, requeued payloads included.
pub struct Consumer<M> {
    rx: Input<M>,
    shared: Arc<Shared<M>>,
}

impl<M> Clone for Consumer<M> {
    fn clone(&self) -> Self {
        Self {
            rx: self.rx.clone(),
            shared: self.shared.clone(),
        }
    }
}

impl<M: Message> Consumer<M> {
    pub(super) fn new(
        topic: String,
        sender: &flume::Sender<M>,
        rx: flume::Receiver<M>,
        disk: Option<DiskWriter>,
    ) -> Self {
        Self::with_input(
            topic,
            Input::Plain(rx),
            WeakInput::Plain(sender.downgrade()),
            disk,
        )
    }

    /// A consumer of a write-through topic, whose deliveries stay on disk
    /// until acked.
    pub(super) fn leased(
        topic: String,
        sender: &flume::Sender<Leased<M>>,
        rx: flume::Receiver<Leased<M>>,
        disk: DiskWriter,
    ) -> Self {
        Self::with_input(
            topic,
            Input::Leased(rx),
            WeakInput::Leased(sender.downgrade()),
            Some(disk),
        )
    }

    fn with_input(
        topic: String,
        rx: Input<M>,
        weak: WeakInput<M>,
        disk: Option<DiskWriter>,
    ) -> Self {
        Self {
            rx,
            shared: Arc::new(Shared {
                topic,
                weak,
                disk,
                stash: Mutex::new(VecDeque::new()),
                stashed: AtomicUsize::new(0),
                notify: Notify::new(),
            }),
        }
    }

    /// A consumer over a bare flume channel, for driving a sink without a
    /// topic. Keeps only a weak handle on `sender`, so dropping every other
    /// sender still disconnects it.
    pub fn from_flume(sender: &flume::Sender<M>, rx: flume::Receiver<M>) -> Self {
        Self::new(String::from("flume"), sender, rx, None)
    }

    /// The bare input, for a reader that never requeues. `None` for a
    /// write-through topic, whose keys only an ack deletes.
    pub(super) fn into_receiver(self) -> Option<flume::Receiver<M>> {
        match self.rx {
            Input::Plain(rx) => Some(rx),
            Input::Leased(_) => None,
        }
    }

    fn pop_stash(&self) -> Option<Delivery<M>> {
        if self.shared.stashed.load(Ordering::Relaxed) == 0 {
            return None;
        }
        let mut stash = self.shared.stash.lock();
        let entry = stash.pop_front();
        self.shared.stashed.store(stash.len(), Ordering::Relaxed);
        entry.map(|(key, msg)| Delivery::new(key, msg))
    }

    fn push_stash(&self, key: Option<u64>, msg: M) {
        {
            let mut stash = self.shared.stash.lock();
            stash.push_back((key, msg));
            self.shared.stashed.store(stash.len(), Ordering::Relaxed);
        }
        self.shared.notify.notify_one();
    }

    /// The next payload: a requeued one first, then the input. `None` once
    /// the input is disconnected and nothing is left requeued.
    ///
    /// Cancel-safe: a payload is only taken in the poll that returns it.
    pub async fn recv(&self) -> Option<Delivery<M>> {
        loop {
            if let Some(delivery) = self.pop_stash() {
                return Some(delivery);
            }
            // A requeue that lands between the check above and parking
            // leaves a permit in `notify`, so this cannot sleep through it.
            tokio::select! {
                biased;

                entry = self.rx.recv() => {
                    return match entry {
                        Some((key, msg)) => Some(Delivery::new(key, msg)),
                        // Requeued after the check above, and the input has
                        // nothing left to wake us with.
                        None => self.pop_stash(),
                    };
                }
                _ = self.shared.notify.notified() => {}
            }
        }
    }

    /// Settles a payload the consumer is done with, whatever the outcome. On
    /// a write-through topic this deletes its key, off the async thread.
    pub fn ack(&self, token: AckToken) {
        self.ack_all(std::iter::once(token));
    }

    pub fn ack_all(&self, tokens: impl IntoIterator<Item = AckToken>) {
        let keys: Vec<u64> = tokens.into_iter().filter_map(|token| token.key).collect();
        if keys.is_empty() {
            return;
        }
        let Some(disk) = &self.shared.disk else {
            return;
        };
        disk.remove(keys);
    }

    /// Hands a payload back for redelivery, to this or any other worker of
    /// the topic. Never waits on a reader, so a worker can requeue while every
    /// other worker is busy, or none is left.
    ///
    /// A disk topic writes it back to its keyspace, skipping the
    /// `max_disk_size` wait: only the consumer itself could free that space.
    /// A write-through topic still holds its key, so only the in-memory copy
    /// is requeued: writing it again would deliver it twice.
    pub async fn nack(&self, delivery: Delivery<M>) {
        let (msg, AckToken { key }) = delivery.into_parts();
        self.requeue(key, msg).await;
    }

    /// Takes over a payload the consumer of the topic this one replaced
    /// (`from`) still held, settling it there. A write-through lease moves
    /// over as is when this topic is write-through too (same keyspace); any
    /// other payload is queued here as new, and only then acked on `from`, so
    /// a crash in between duplicates it rather than losing it.
    pub async fn adopt(&self, from: &Consumer<M>, delivery: Delivery<M>) {
        let (msg, token) = delivery.into_parts();
        if token.key.is_some() && matches!(self.rx, Input::Leased(_)) {
            self.requeue(token.key, msg).await;
            return;
        }
        self.requeue(None, msg).await;
        from.ack(token);
    }

    async fn requeue(&self, key: Option<u64>, msg: M) {
        if key.is_none()
            && let Some(disk) = &self.shared.disk
            && disk.takes_requeues()
        {
            match disk.write(&msg).await {
                Ok(()) => return,
                Err(e) => {
                    warn!(error = %format!("{e:#}"), topic = %self.shared.topic, "failed to requeue payload to disk")
                }
            }
        }
        // Full: the input holds payloads, or (with no buffer) no worker is
        // parked on it; either way the next `recv` checks the stash first.
        // Disconnected: `recv` drains the stash before it reports the end.
        if let Err((key, msg)) = self.shared.weak.try_send(key, msg) {
            self.push_stash(key, msg);
        }
    }

    /// Requeued payloads waiting in the stash.
    pub fn stashed(&self) -> usize {
        self.shared.stashed.load(Ordering::Relaxed)
    }
}
