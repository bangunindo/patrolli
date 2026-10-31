// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::InternalMetrics;
use super::codec::{self, Codec};
use super::consumer::{Consumer, Leased};
use crate::common::internal_metrics::{self, ChannelKind, ChannelState};
use crate::{common::HasLength, config};
use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use bytes::Bytes;
use fjall::{Database, Keyspace, KeyspaceCreateOptions, KvSeparationOptions};
use futures::future::join_all;
use parking_lot::Mutex as ParkingMutex;
use petgraph::matrix_graph::Zero;
use rustc_hash::FxHashMap as HashMap;
use std::fmt::Display;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, OnceLock, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

static DATABASE: OnceLock<Database> = OnceLock::new();

/// How often a sync send waiting on a full topic checks whether it should
/// stop waiting.
const STOP_POLL: Duration = Duration::from_millis(100);

pub fn init_database(conf: &config::global::DiskChannel) -> Result<()> {
    let mut builder = Database::builder(&conf.data_dir);
    if conf.cache_size > 0 {
        builder = builder.cache_size(conf.cache_size);
    }
    if conf.max_journal_size > 0 {
        builder = builder.max_journaling_size(conf.max_journal_size);
    }
    let db = builder.open().context("Failed to open database")?;
    _ = DATABASE.set(db);
    Ok(())
}

/// Deletes every keyspace of the disk channel database that `orphaned` picks
/// by name, and returns the names it deleted. For data no topic will open
/// again: a topic still open over one keeps it until its last handle drops.
pub async fn remove_keyspaces(
    orphaned: impl Fn(&str) -> bool + Send + 'static,
) -> Result<Vec<String>> {
    let Some(db) = DATABASE.get() else {
        return Ok(Vec::new());
    };
    tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
        let mut removed = Vec::new();
        for name in db.list_keyspace_names() {
            if !orphaned(&name) {
                continue;
            }
            let keyspace = db.keyspace(&name, KeyspaceCreateOptions::default)?;
            db.delete_keyspace(keyspace)?;
            // A topic opening the name again starts over, rather than share
            // the cursors of a keyspace that is going away.
            OPEN_KEYSPACES.lock().remove(&*name);
            removed.push(name.to_string());
        }
        Ok(removed)
    })
    .await
    .context("keyspace removal task failed")?
}

pub trait Topic: AsRef<[u8]> + Clone + Display + Ord + Eq + Hash + Send + Sync + 'static {}
impl<T: AsRef<[u8]> + Clone + Display + Ord + Eq + Hash + Send + Sync + 'static> Topic for T {}

pub trait Message: Codec + HasLength + Clone + Send + Sync + 'static {}
impl<M: Codec + HasLength + Clone + Send + Sync + 'static> Message for M {}

#[derive(Debug, thiserror::Error)]
pub enum Error<M> {
    #[error("Topic already exists")]
    TopicAlreadyExists,
    #[error("Topic not found")]
    TopicNotFound(Option<M>),
    #[error("Channel buffer is full")]
    BufferFull(M),
    #[error("Channel is closed")]
    Closed(M),
    #[error("Internal error: {0}")]
    Internal(anyhow::Error),
    #[error("Multiple errors occurred")]
    Multiple(Vec<Error<M>>),
}

#[derive(Clone)]
struct Memory<T, M> {
    topic: T,
    conf: config::MemoryChannel,

    sender: flume::Sender<M>,
}

impl<T: Topic, M: Message> Memory<T, M> {
    fn try_send(&self, payload: M) -> Result<(), Error<M>> {
        match self.sender.try_send(payload) {
            Ok(()) => Ok(()),
            Err(flume::TrySendError::Full(payload)) => Err(Error::BufferFull(payload)),
            Err(flume::TrySendError::Disconnected(payload)) => Err(Error::Closed(payload)),
        }
    }

    async fn send(&self, payload: M) -> Result<(), Error<M>> {
        if self.conf.non_blocking {
            self.try_send(payload)
        } else {
            match self.sender.send_async(payload).await {
                Ok(()) => Ok(()),
                Err(flume::SendError(payload)) => Err(Error::Closed(payload)),
            }
        }
    }

    fn send_sync(&self, payload: M, stop: Option<&CancellationToken>) -> Result<(), Error<M>> {
        if self.conf.non_blocking {
            return self.try_send(payload);
        }
        let Some(stop) = stop else {
            return match self.sender.send(payload) {
                Ok(()) => Ok(()),
                Err(flume::SendError(payload)) => Err(Error::Closed(payload)),
            };
        };
        let mut payload = payload;
        loop {
            if stop.is_cancelled() {
                return self.try_send(payload);
            }
            match self.sender.send_timeout(payload, STOP_POLL) {
                Ok(()) => return Ok(()),
                Err(flume::SendTimeoutError::Timeout(p)) => payload = p,
                Err(flume::SendTimeoutError::Disconnected(p)) => return Err(Error::Closed(p)),
            }
        }
    }
}

/// Where a keyspace is written and read, and how much it holds. While a
/// topic winds down, the next one over its keyspace shares its write cursor
/// and `held`, so the two never reuse a key or count one twice; a
/// replacement (see [`AdminChannel::replace`]) also shares the read cursor,
/// since the old topic's leases are handed over rather than read again.
#[derive(Clone)]
struct Cursors {
    /// Next key to write. Held across assigning the key *and* inserting it,
    /// so keys become visible in order and `read` never passes one that is
    /// still being written.
    write: Arc<ParkingMutex<u64>>,
    /// No live key is below this, except keys leased by a write-through
    /// topic. A read seeks here rather than to the start: every pop leaves a
    /// tombstone, and a scan from the start steps over all of them, which
    /// made a drain quadratic in the backlog.
    read: Arc<ParkingMutex<u64>>,
    /// Bytes of the values the keyspace holds, leased ones included: what
    /// `max_disk_size` caps. Not `Keyspace::disk_space`, which only shrinks
    /// once a flush and a compaction drop what was deleted, and a flush needs
    /// writes, the very thing a full topic holds back: gating on it stalled a
    /// drained topic for good.
    held: Arc<AtomicU64>,
}

impl Cursors {
    /// The cursors of a keyspace no open topic shares, holding `held` bytes,
    /// with writes going on at `write`.
    fn new(write: u64, held: u64) -> Self {
        Self {
            write: Arc::new(ParkingMutex::new(write)),
            read: fresh_read_cursor(),
            held: Arc::new(AtomicU64::new(held)),
        }
    }

    /// The cursors of a topic still open over this keyspace, shared with the
    /// topic opening it (the read cursor too when `replacing`), or `None`
    /// when there is none and the keyspace has to be counted for
    /// [`open`](Self::open).
    fn shared(topic: &str, replacing: bool) -> Option<Self> {
        let mut open = OPEN_KEYSPACES.lock();
        let cursors = Self::live(&mut open, topic, replacing)?;
        Self::register(&mut open, topic.to_string(), &cursors);
        Some(cursors)
    }

    /// The cursors for a topic opening its keyspace: shared with a topic
    /// over it still open, else fresh, with writes going on after `last_key`.
    fn open(topic: String, last_key: u64, held: u64, replacing: bool) -> Self {
        let mut open = OPEN_KEYSPACES.lock();
        let cursors = Self::live(&mut open, &topic, replacing)
            .unwrap_or_else(|| Self::new(last_key + 1, held));
        Self::register(&mut open, topic, &cursors);
        cursors
    }

    fn live(open: &mut HashMap<String, WeakCursors>, topic: &str, replacing: bool) -> Option<Self> {
        open.retain(|_, (write, _, _)| write.strong_count() > 0);
        let (write, read, held) = open.get(topic)?;
        Some(Self {
            write: write.upgrade()?,
            read: read
                .upgrade()
                .filter(|_| replacing)
                .unwrap_or_else(fresh_read_cursor),
            held: held.upgrade()?,
        })
    }

    fn register(open: &mut HashMap<String, WeakCursors>, topic: String, cursors: &Self) {
        open.insert(
            topic,
            (
                Arc::downgrade(&cursors.write),
                Arc::downgrade(&cursors.read),
                Arc::downgrade(&cursors.held),
            ),
        );
    }

    /// Takes `bytes` deleted from the keyspace off `held`. Saturating: a
    /// count that drifted low only lets the topic take more, never wraps
    /// into a topic that is full for good.
    fn release(&self, bytes: u64) {
        _ = self
            .held
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
                Some(held.saturating_sub(bytes))
            });
    }
}

/// The first pop of a reopened topic seeks past the tombstones of the
/// previous run once, then moves on.
fn fresh_read_cursor() -> Arc<ParkingMutex<u64>> {
    Arc::new(ParkingMutex::new(0))
}

type WeakCursors = (
    Weak<ParkingMutex<u64>>,
    Weak<ParkingMutex<u64>>,
    Weak<AtomicU64>,
);

/// The cursors of every keyspace a topic has open, by topic name.
static OPEN_KEYSPACES: LazyLock<ParkingMutex<HashMap<String, WeakCursors>>> =
    LazyLock::new(Default::default);

/// Appends to a disk topic's keyspace: the bridge spilling what the consumer
/// had no room for, and the consumer requeueing what it took.
#[derive(Clone)]
pub(super) struct DiskWriter {
    keyspace: Keyspace,
    compression: config::OtapCompression,
    transport_optimized: bool,
    cursors: Cursors,
    /// Set once the topic is removed: its keyspace is going away, so
    /// nothing more is written to it.
    delete_on_exit: Arc<AtomicBool>,
    /// Set once a switch to a memory channel leaves the keyspace behind:
    /// nothing reads it until the topic is a disk one again, so a requeue
    /// stays in memory, where the replacing consumer adopts it.
    left_behind: Arc<AtomicBool>,
    /// Wakes a bridge sender idling on an empty keyspace.
    written: Arc<Notify>,
}

impl DiskWriter {
    fn new(
        keyspace: Keyspace,
        conf: &config::DiskChannel,
        cursors: Cursors,
        delete_on_exit: Arc<AtomicBool>,
    ) -> Self {
        Self {
            keyspace,
            compression: conf.compression,
            transport_optimized: conf.transport_optimized,
            cursors,
            delete_on_exit,
            left_behind: Arc::new(AtomicBool::new(false)),
            written: Arc::new(Notify::new()),
        }
    }

    /// Whether a consumer's requeue goes back to the keyspace: not once it
    /// is deleted, nor once it is left behind unread.
    pub(super) fn takes_requeues(&self) -> bool {
        !self.delete_on_exit.load(Ordering::Relaxed) && !self.left_behind.load(Ordering::Relaxed)
    }

    fn insert(keyspace: &Keyspace, cursors: &Cursors, value: Bytes) -> Result<()> {
        let size = value.len() as u64;
        let mut cursor = cursors.write.lock();
        // Counted before the key is visible, so a reader deleting it at once
        // never releases bytes that were not counted yet.
        cursors.held.fetch_add(size, Ordering::Relaxed);
        if let Err(e) = keyspace.insert(Bytes::from_owner(cursor.to_be_bytes()), value) {
            cursors.release(size);
            return Err(e).context("failed to insert into keyspace");
        }
        *cursor += 1;
        Ok(())
    }

    /// Bytes the keyspace holds, see [`Cursors::held`].
    fn held(&self) -> u64 {
        self.cursors.held.load(Ordering::Relaxed)
    }

    pub(super) async fn write<M: Message>(&self, payload: &M) -> Result<()> {
        // Encoded off the async thread too: compressing a large payload
        // takes milliseconds. The clone shares its Arrow buffers.
        let payload = payload.clone();
        let (compression, transport_optimized) = (self.compression, self.transport_optimized);
        let keyspace = self.keyspace.clone();
        let cursors = self.cursors.clone();
        tokio::task::spawn_blocking(move || {
            let value = codec::encode(&payload, compression, transport_optimized)?;
            Self::insert(&keyspace, &cursors, value)
        })
        .await
        .context("disk write task failed")??;
        self.written.notify_one();
        Ok(())
    }

    /// [`write`](Self::write) for a caller that may block: the processor's
    /// sync send path.
    fn write_blocking<M: Codec>(&self, payload: &M) -> Result<()> {
        let value = codec::encode(payload, self.compression, self.transport_optimized)?;
        Self::insert(&self.keyspace, &self.cursors, value)?;
        self.written.notify_one();
        Ok(())
    }

    /// Deletes acked keys of a write-through topic, off the async thread. A
    /// delete that fails or never runs only brings the payload back on the
    /// next start.
    pub(super) fn remove(&self, keys: Vec<u64>) {
        let keyspace = self.keyspace.clone();
        let cursors = self.cursors.clone();
        let delete_on_exit = self.delete_on_exit.clone();
        let remove = move || {
            // A removed topic's keyspace is deleted whole.
            if delete_on_exit.load(Ordering::Relaxed) {
                return;
            }
            let mut batch = DATABASE.get().expect("disk channel database").batch();
            let mut freed = 0;
            for key in keys {
                let key = Bytes::from_owner(key.to_be_bytes());
                // Read from the index, the value stays where it is. A key
                // already gone frees nothing.
                if let Ok(Some(size)) = keyspace.size_of(&key) {
                    freed += u64::from(size);
                }
                batch.remove(&keyspace, key);
            }
            match batch.commit() {
                Ok(()) => cursors.release(freed),
                Err(e) => warn!(error = %e, "failed to delete acked payloads from disk"),
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn_blocking(remove);
            }
            Err(_) => remove(),
        }
    }
}

/// How often a disk topic checks whether its files need compacting. Short in
/// tests, which compact a few kilobytes.
const COMPACT_INTERVAL: Duration = if cfg!(test) {
    Duration::from_millis(100)
} else {
    Duration::from_secs(5)
};
/// How far past what a topic holds its files may grow before a compaction:
/// more than this, and than a quarter of what it holds.
const COMPACT_SLACK: u64 = if cfg!(test) { 0 } else { 64 * 1024 * 1024 };

#[derive(Clone)]
struct DiskBridge<T, M> {
    topic: T,
    conf: config::DiskChannel,
    keyspace: Keyspace,
    writer: DiskWriter,
    delete_on_exit: Arc<AtomicBool>,
    worker_count: Arc<AtomicUsize>,
    sender: BridgeOutput<M>,
    receiver: flume::Receiver<M>,
    /// The topic's, for what is lost on the way through the keyspace.
    metrics: Arc<InternalMetrics<T>>,
}

/// Where the bridge hands what it read off disk: the consumer's input.
enum BridgeOutput<M> {
    /// A read deletes the key, so the payload lives on in memory only.
    Plain(flume::Sender<M>),
    /// Write-through: a read only leases the key, the consumer's ack deletes
    /// it.
    Leased(flume::Sender<Leased<M>>),
}

impl<M> Clone for BridgeOutput<M> {
    fn clone(&self) -> Self {
        match self {
            BridgeOutput::Plain(sender) => BridgeOutput::Plain(sender.clone()),
            BridgeOutput::Leased(sender) => BridgeOutput::Leased(sender.clone()),
        }
    }
}

impl<M> BridgeOutput<M> {
    fn is_leased(&self) -> bool {
        matches!(self, BridgeOutput::Leased(_))
    }
}

impl<T: Topic, M: Message> DiskBridge<T, M> {
    async fn run(self) {
        let this = Arc::new(self);
        // Write-through producers write to disk themselves.
        if !this.sender.is_leased() {
            for _ in 0..this.conf.worker_count {
                let this = this.clone();
                tokio::spawn(async move {
                    this.receiver().await;
                });
            }
        }
        for _ in 0..this.conf.worker_count {
            let this = this.clone();
            tokio::spawn(async move {
                this.sender().await;
            });
        }
        tokio::spawn(Self::compactor(Arc::downgrade(&this)));
    }

    /// Compacts the keyspace once its files hold well past its payloads.
    /// fjall drops a delivered payload's blob file only when a compaction
    /// merges the delete with the entry pointing into it, and on its own it
    /// compacts after a few memtable flushes (gigabytes of writes at the
    /// default memtable size), or never once writes stop. Weak: the bridge
    /// owns the consumer's input, which must end with its senders.
    async fn compactor(this: Weak<Self>) {
        // A compaction only frees the files of the versions before the last
        // one: the next one deletes what this one dropped.
        let mut follow_up = false;
        loop {
            sleep(COMPACT_INTERVAL).await;
            let Some(bridge) = this.upgrade() else {
                break;
            };
            if bridge.delete_on_exit.load(Ordering::Relaxed) {
                break;
            }
            let held = bridge.writer.held();
            let on_disk = bridge.keyspace.disk_space();
            let excess = on_disk.saturating_sub(held);
            let due = excess > COMPACT_SLACK && excess > held / 4;
            if !due && !std::mem::take(&mut follow_up) {
                continue;
            }
            follow_up = due;
            let keyspace = bridge.keyspace.clone();
            drop(bridge);
            let started = Instant::now();
            let compacted = tokio::task::spawn_blocking(move || -> fjall::Result<u64> {
                // fjall prunes old versions, deleting the files only they
                // hold, when it rotates a memtable, and skips an empty one:
                // a tombstone under every key a topic writes (they start at
                // 1) makes sure there is one to rotate.
                keyspace.remove(Bytes::from_owner(0u64.to_be_bytes()))?;
                // Flushed first: the deletes still in the memtable are what
                // the compaction needs to see.
                keyspace.rotate_memtable_and_wait()?;
                keyspace.major_compact()?;
                Ok(keyspace.disk_space())
            })
            .await;
            match compacted {
                Ok(Ok(after)) => debug!(
                    held,
                    before = on_disk,
                    after,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "disk channel compacted"
                ),
                Ok(Err(e)) => warn!(error = %e, "disk channel compaction failed"),
                Err(e) => warn!(error = %e, "disk channel compaction task failed"),
            }
        }
    }

    /// The oldest key at or past the read cursor, decoded. A write-through
    /// topic leaves the key for the ack to delete, every other topic deletes
    /// it here. A key or a value that does not decode is deleted, or it
    /// would stall the cursor, and counted lost; the read goes on past it.
    async fn get_from_disk(&self) -> Result<Option<(u64, M)>, Error<M>> {
        let keyspace = self.keyspace.clone();
        let cursors = self.writer.cursors.clone();
        let lease = self.sender.is_leased();
        let (topic, metrics) = (self.topic.clone(), self.metrics.clone());
        let internal = |e: fjall::Error| Error::Internal(e.into());
        tokio::task::spawn_blocking(move || {
            loop {
                let (index, key, value) = {
                    let mut cursor = cursors.read.lock();
                    let Some(kv) = keyspace.range(cursor.to_be_bytes()..).next() else {
                        return Ok(None);
                    };
                    let (key, value) = kv.into_inner().map_err(internal)?;
                    let Ok(index) = <[u8; 8]>::try_from(key.as_ref()).map(u64::from_be_bytes)
                    else {
                        // Not a key a topic writes.
                        keyspace.remove(key).map_err(internal)?;
                        cursors.release(value.len() as u64);
                        metrics.add_batches_lost(&topic, 1);
                        error!(topic = %topic, "deleted a disk channel key that is not one");
                        continue;
                    };
                    if !lease {
                        keyspace.remove(key.clone()).map_err(internal)?;
                        cursors.release(value.len() as u64);
                    }
                    *cursor = index + 1;
                    (index, key, value)
                };
                // Past the cursor lock, so the bridge's senders decode in
                // parallel.
                let size = value.len() as u64;
                match codec::decode(value.into()) {
                    Ok(msg) => return Ok(Some((index, msg))),
                    Err(e) => {
                        if lease {
                            keyspace.remove(key).map_err(internal)?;
                            cursors.release(size);
                        }
                        metrics.add_batches_lost(&topic, 1);
                        error!(
                            error = %format!("{e:#}"),
                            topic = %topic,
                            "deleted a disk channel value that does not decode"
                        );
                    }
                }
            }
        })
        .await
        .map_err(|e| Error::Internal(e.into()))?
    }

    async fn dump_to_disk(&self, payload: &M) -> Result<(), Error<M>> {
        self.writer.write(payload).await.map_err(Error::Internal)
    }

    /// Counts a payload the topic took and then could not keep.
    fn lost(&self, payload: &M) {
        self.metrics.add_batches_lost(&self.topic, 1);
        self.metrics.add_messages_lost(&self.topic, payload.len());
    }

    #[tracing::instrument(skip_all, fields(topic = %self.topic))]
    async fn sender(self: Arc<Self>) {
        debug!("disk bridge sender started");
        self.worker_count.fetch_add(1, Ordering::Relaxed);
        loop {
            if self.receiver.is_disconnected() {
                break;
            }
            match self.get_from_disk().await {
                Ok(Some((key, payload))) => {
                    let gone = match &self.sender {
                        BridgeOutput::Plain(sender) => {
                            match sender.send_async(payload).await {
                                Ok(()) => false,
                                // Its key is already deleted: write it back.
                                Err(flume::SendError(payload)) => {
                                    if let Err(e) = self.dump_to_disk(&payload).await {
                                        error!(error = %e, "failed to dump payload to disk");
                                        self.lost(&payload);
                                    }
                                    true
                                }
                            }
                        }
                        // The key stays until acked, so it comes back on the
                        // next start.
                        BridgeOutput::Leased(sender) => {
                            sender.send_async((key, payload)).await.is_err()
                        }
                    };
                    if gone {
                        debug!("disk channel consumer gone");
                        break;
                    }
                }
                Ok(None) => {
                    // A write wakes one idle sender; the timeout covers the
                    // others, and the input disconnecting.
                    tokio::select! {
                        _ = sleep(Duration::from_millis(100)) => {}
                        _ = self.writer.written.notified() => {}
                    }
                }
                Err(e) => {
                    error!(error = %e, "failed to get payload from disk");
                    // Paced: a keyspace that keeps failing would spin.
                    sleep(STOP_POLL).await;
                }
            }
        }
        let worker_count = self.worker_count.fetch_sub(1, Ordering::Relaxed);
        if worker_count == 1
            && self.delete_on_exit.load(Ordering::Relaxed)
            && let Err(e) = DATABASE
                .get()
                .unwrap()
                .delete_keyspace(self.keyspace.clone())
        {
            warn!(error = %e, "failed to delete disk channel keyspace");
        }
        debug!("disk bridge sender stopped");
    }

    #[tracing::instrument(skip_all, fields(topic = %self.topic))]
    async fn receiver(self: Arc<Self>) {
        debug!("disk bridge receiver started");
        let BridgeOutput::Plain(sender) = &self.sender else {
            return;
        };
        while let Ok(payload) = self.receiver.recv_async().await {
            // in case the remote receiver is ready, we skip persistence
            match sender.try_send(payload) {
                Ok(()) => {}
                Err(flume::TrySendError::Full(payload)) => {
                    if let Err(e) = self.dump_to_disk(&payload).await {
                        error!(error = %e, "failed to dump payload to disk");
                        // The consumer takes it once it has room: memory
                        // backpressure rather than a loss.
                        if let Err(flume::SendError(payload)) = sender.send_async(payload).await {
                            self.lost(&payload);
                        }
                    }
                }
                // A send that was on its way when the topic was replaced or
                // removed: the keyspace keeps it for the topic that reads it
                // next, a replacing one included.
                Err(flume::TrySendError::Disconnected(payload)) => {
                    if self.delete_on_exit.load(Ordering::Relaxed) {
                        debug!("payload for a removed disk topic dropped");
                    } else if let Err(e) = self.dump_to_disk(&payload).await {
                        error!(error = %e, "failed to dump payload to disk");
                        self.lost(&payload);
                    }
                }
            }
        }
        debug!("disk bridge receiver stopped");
    }
}

#[derive(Clone)]
struct Disk<T, M> {
    topic: T,
    conf: config::DiskChannel,
    keyspace: Keyspace,
    delete_on_exit: Arc<AtomicBool>,
    /// The bridge's input. A write-through topic sends nothing through it
    /// and holds it only so the bridge sees the topic close.
    sender: flume::Sender<M>,
    /// Set on a write-through topic: a send is done once it is on disk.
    write_through: Option<DiskWriter>,
    /// The topic's writer, read for how much its keyspace holds.
    writer: DiskWriter,
    /// Cancelled once the topic is closed, replaced or removed: a send
    /// waiting on the cap stops waiting, since nothing may ever free it (a
    /// removed topic's consumer is gone).
    closed: CancellationToken,
}

impl<T: Topic, M: Message> Disk<T, M> {
    fn is_full(&self) -> bool {
        !self.conf.max_disk_size.is_zero() && self.writer.held() >= self.conf.max_disk_size
    }

    /// Past the wait on a full topic: a removed topic takes nothing, its
    /// keyspace is going away. Any other topic past `closed` (or `stop`)
    /// takes the payload over its cap.
    fn removed(&self, payload: M) -> Result<M, Error<M>> {
        if self.delete_on_exit.load(Ordering::Relaxed) {
            return Err(Error::Closed(payload));
        }
        Ok(payload)
    }

    async fn send(&self, payload: M) -> Result<(), Error<M>> {
        let mut warned = false;
        while self.is_full() {
            if self.conf.non_blocking {
                return Err(Error::BufferFull(payload));
            }
            if self.closed.is_cancelled() {
                break;
            }
            if !warned {
                warn!(topic = %self.topic, "disk channel full, applying backpressure");
                warned = true;
            }
            tokio::select! {
                _ = sleep(STOP_POLL) => {}
                _ = self.closed.cancelled() => {}
            }
        }
        let payload = self.removed(payload)?;
        if let Some(writer) = &self.write_through {
            return writer.write(&payload).await.map_err(Error::Internal);
        }
        match self.sender.send_async(payload).await {
            Ok(()) => Ok(()),
            Err(flume::SendError(payload)) => Err(Error::Closed(payload)),
        }
    }

    fn send_sync(&self, payload: M, stop: Option<&CancellationToken>) -> Result<(), Error<M>> {
        let mut warned = false;
        while self.is_full() {
            if self.conf.non_blocking {
                return Err(Error::BufferFull(payload));
            }
            // Past `stop` the cap gives way rather than drop: the topic
            // exists to keep what its consumer cannot take yet, and the
            // overshoot is only what the stopping callers still hold.
            if stop.is_some_and(CancellationToken::is_cancelled) || self.closed.is_cancelled() {
                break;
            }
            if !warned {
                warn!(topic = %self.topic, "disk channel full, applying backpressure");
                warned = true;
            }
            std::thread::sleep(STOP_POLL);
        }
        let payload = self.removed(payload)?;
        if let Some(writer) = &self.write_through {
            return writer.write_blocking(&payload).map_err(Error::Internal);
        }
        match self.sender.send(payload) {
            Ok(()) => Ok(()),
            Err(flume::SendError(payload)) => Err(Error::Closed(payload)),
        }
    }
}

#[derive(Clone)]
enum Channel<T, M> {
    Memory {
        chan: Memory<T, M>,
        metrics: Arc<InternalMetrics<T>>,
    },
    Disk {
        chan: Disk<T, M>,
        metrics: Arc<InternalMetrics<T>>,
    },
}

impl<T: Topic, M: Message> Channel<T, M> {
    async fn new(
        topic: T,
        internal_metrics: Arc<InternalMetrics<T>>,
        conf: config::Channel,
        replacing: bool,
    ) -> Result<(Self, Consumer<M>), Error<M>> {
        match conf {
            config::Channel::Memory(conf) => {
                let (sender, receiver) = flume::bounded(conf.channel_buffer_size);
                let consumer = Consumer::new(topic.to_string(), &sender, receiver, None);
                Ok((
                    Self::Memory {
                        chan: Memory {
                            topic,
                            conf,
                            sender,
                        },
                        metrics: internal_metrics,
                    },
                    consumer,
                ))
            }
            config::Channel::Disk(conf) => {
                let (sender1, receiver1) = flume::bounded(0);
                let max_memtable_size = conf.max_memtable_size;
                let name = topic.to_string();
                let db = DATABASE.get().unwrap().clone();
                // A topic still open over the keyspace (a disk topic being
                // replaced) already counts it, and is writing to it.
                let shared = Cursors::shared(&name, replacing);
                let in_use = shared.is_some();
                let (keyspace, last_key, held) = tokio::task::spawn_blocking(move || {
                    let mut options = KeyspaceCreateOptions::default()
                        .with_kv_separation(Some(KvSeparationOptions::default()));
                    if max_memtable_size > 0 {
                        options = options.max_memtable_size(max_memtable_size);
                    }
                    let keyspace = db
                        .keyspace(&name, || options)
                        .map_err(|e| Error::Internal(e.into()))?;
                    if in_use {
                        return Ok((keyspace, 0, 0));
                    }
                    // Writes go on after the last key a topic wrote. A key
                    // of any other length is not one, and must not cost the
                    // backlog: the reader deletes it when it gets there.
                    let last_key = keyspace
                        .iter()
                        .rev()
                        .find_map(|kv| {
                            kv.key()
                                .ok()
                                .and_then(|key| <[u8; 8]>::try_from(key.as_ref()).ok())
                        })
                        .map(u64::from_be_bytes)
                        .unwrap_or_default();
                    // What a previous run left. Sizes come from the index,
                    // values are not read.
                    let held = keyspace
                        .iter()
                        .map(|kv| kv.size().map(u64::from))
                        .sum::<Result<u64, _>>()
                        .map_err(|e| Error::Internal(e.into()))?;
                    Ok((keyspace, last_key, held))
                })
                .await
                .unwrap()?;
                let cursors = shared
                    .unwrap_or_else(|| Cursors::open(topic.to_string(), last_key, held, replacing));
                let delete_on_exit = Arc::new(AtomicBool::new(false));
                let writer =
                    DiskWriter::new(keyspace.clone(), &conf, cursors, delete_on_exit.clone());
                let disk = Disk {
                    topic: topic.clone(),
                    conf,
                    keyspace: keyspace.clone(),
                    delete_on_exit: delete_on_exit.clone(),
                    sender: sender1,
                    write_through: conf.write_through.then(|| writer.clone()),
                    writer: writer.clone(),
                    closed: CancellationToken::new(),
                };
                let (consumer, sender2) = if conf.write_through {
                    let (sender, receiver) = flume::bounded(conf.channel_buffer_size);
                    let consumer =
                        Consumer::leased(topic.to_string(), &sender, receiver, writer.clone());
                    (consumer, BridgeOutput::Leased(sender))
                } else {
                    let (sender, receiver) = flume::bounded(conf.channel_buffer_size);
                    let consumer =
                        Consumer::new(topic.to_string(), &sender, receiver, Some(writer.clone()));
                    (consumer, BridgeOutput::Plain(sender))
                };
                let disk_bridge = DiskBridge {
                    topic,
                    conf,
                    keyspace,
                    writer,
                    delete_on_exit,
                    worker_count: Arc::new(AtomicUsize::new(0)),
                    receiver: receiver1,
                    sender: sender2,
                    metrics: internal_metrics.clone(),
                };
                disk_bridge.run().await;
                Ok((
                    Self::Disk {
                        chan: disk,
                        metrics: internal_metrics,
                    },
                    consumer,
                ))
            }
        }
    }

    fn topic(&self) -> &T {
        match self {
            Channel::Memory { chan, .. } => &chan.topic,
            Channel::Disk { chan, .. } => &chan.topic,
        }
    }

    fn state(&self) -> ChannelState {
        match self {
            Channel::Memory { chan, .. } => ChannelState {
                kind: ChannelKind::Memory,
                capacity: chan.sender.capacity().unwrap_or_default() as u64,
                length: chan.sender.len() as u64,
            },
            Channel::Disk { chan, .. } => ChannelState {
                kind: ChannelKind::Disk,
                capacity: chan.conf.max_disk_size,
                length: chan.keyspace.disk_space(),
            },
        }
    }

    /// Takes the topic out of service: a send still waiting on its cap stops
    /// waiting. Producers already moved on with the collection.
    fn close(&self) {
        // A memory topic's waiting send ends with its consumer.
        if let Channel::Disk { chan, .. } = self {
            chan.closed.cancel();
        }
    }

    fn remove(self) -> Result<()> {
        match self {
            // memory channel doesn't have any cleanup, we just drop it
            Channel::Memory { .. } => {}
            Channel::Disk { chan, .. } => {
                // Before `closed`, so a send it releases sees the removal.
                chan.delete_on_exit.store(true, Ordering::Relaxed);
                chan.closed.cancel();
            }
        }
        Ok(())
    }

    fn track(
        &self,
        im: &InternalMetrics<T>,
        n: Instant,
        len: usize,
        res: Result<(), Error<M>>,
    ) -> Result<(), Error<M>> {
        im.add_total_batches(1);
        im.add_total_messages(len);
        im.add_send_wait_time(self.topic(), n.elapsed());
        if res.is_ok() {
            im.add_batches_sent(self.topic(), 1);
            im.add_messages_sent(self.topic(), len);
        } else {
            im.add_batches_dropped(self.topic(), 1);
            im.add_messages_dropped(self.topic(), len);
        }
        res
    }

    async fn send(&self, payload: M) -> Result<(), Error<M>> {
        let n = Instant::now();
        let len = payload.len();
        let (res, im) = match self {
            Channel::Memory { chan, metrics: im } => (chan.send(payload).await, im),
            Channel::Disk { chan, metrics: im } => (chan.send(payload).await, im),
        };
        self.track(im, n, len, res)
    }

    fn send_sync(&self, payload: M, stop: Option<&CancellationToken>) -> Result<(), Error<M>> {
        let n = Instant::now();
        let len = payload.len();
        let (res, im) = match self {
            Channel::Memory { chan, metrics: im } => (chan.send_sync(payload, stop), im),
            Channel::Disk { chan, metrics: im } => (chan.send_sync(payload, stop), im),
        };
        self.track(im, n, len, res)
    }
}

#[derive(Clone)]
struct Collection<T, M> {
    channels: Vec<Channel<T, M>>,
    internal_metrics: Arc<InternalMetrics<T>>,
}

impl<T: Topic, M: Message> Collection<T, M> {
    fn new(m: Arc<InternalMetrics<T>>) -> Self {
        Self {
            channels: Vec::new(),
            internal_metrics: m,
        }
    }

    fn add(&mut self, channel: Channel<T, M>) -> Result<(), Error<M>> {
        match self
            .channels
            .binary_search_by(|other| other.topic().cmp(channel.topic()))
        {
            Ok(_) => Err(Error::TopicAlreadyExists),
            Err(index) => {
                self.channels.insert(index, channel);
                Ok(())
            }
        }
    }

    fn get(&self, topic: &T) -> Option<&Channel<T, M>> {
        self.channels
            .binary_search_by(|channel| channel.topic().cmp(topic))
            .ok()
            .map(|index| &self.channels[index])
    }

    fn remove(&mut self, topic: &T, clear_data: bool) -> Result<(), Error<M>> {
        match self
            .channels
            .binary_search_by(|channel| channel.topic().cmp(topic))
        {
            Ok(index) => {
                let chan = self.channels.remove(index);
                if clear_data {
                    chan.remove().map_err(Error::Internal)?;
                } else {
                    chan.close();
                }
                Ok(())
            }
            Err(_) => Err(Error::TopicNotFound(None)),
        }
    }

    fn remove_and_clear_all(&mut self) -> Result<(), Error<M>> {
        let channels = std::mem::take(&mut self.channels);
        let mut errors = Vec::new();
        for chan in channels {
            if let Err(e) = chan.remove() {
                errors.push(Error::Internal(e));
            };
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::Multiple(errors))
        }
    }

    async fn broadcast(&self, payload: M) -> Result<(), Error<M>> {
        // Nothing took it: the caller decides what becomes of it.
        if self.channels.is_empty() {
            return Err(Error::TopicNotFound(Some(payload)));
        }
        let n = Instant::now();
        // one topic channel is very common, so we optimize for that case
        let res = if self.channels.len() == 1 {
            self.channels[0].send(payload).await
        } else {
            let mut futs = Vec::with_capacity(self.channels.len());
            let (last_chan, rest_chan) = self.channels.split_last().unwrap();
            for chan in rest_chan {
                futs.push(chan.send(payload.clone()));
            }
            futs.push(last_chan.send(payload));
            let mut results = Vec::with_capacity(futs.len());
            let result = join_all(futs).await;
            for r in result {
                if let Err(e) = r {
                    results.push(e);
                }
            }
            if !results.is_empty() {
                Err(Error::Multiple(results))
            } else {
                Ok(())
            }
        };
        self.internal_metrics.add_total_time(n.elapsed());
        res
    }

    fn send_to(
        &self,
        topic: &T,
        payload: M,
        stop: Option<&CancellationToken>,
    ) -> Result<(), Error<M>> {
        let n = Instant::now();
        let Some(chan) = self.get(topic) else {
            self.internal_metrics.add_total_batches(1);
            self.internal_metrics.add_batches_dropped(topic, 1);
            self.internal_metrics.add_total_messages(payload.len());
            self.internal_metrics
                .add_messages_dropped(topic, payload.len());
            self.internal_metrics.add_total_time(n.elapsed());
            return Err(Error::TopicNotFound(Some(payload)));
        };
        let res = chan.send_sync(payload, stop);
        self.internal_metrics.add_total_time(n.elapsed());
        res
    }
}

struct ChannelGroup<T, M> {
    inner: ArcSwap<Collection<T, M>>,
    admin_lock: Mutex<()>,

    internal_metrics: Arc<InternalMetrics<T>>,
    remote_metrics: Arc<ArcSwap<(Instant, internal_metrics::Channel)>>,
    update_interval: Duration,
}

impl<T: Topic, M: Message> ChannelGroup<T, M> {
    fn new(
        update_interval: Duration,
        remote_metrics: Arc<ArcSwap<(Instant, internal_metrics::Channel)>>,
    ) -> Arc<Self> {
        let internal_metrics = Arc::new(InternalMetrics::default());
        let im_clone = internal_metrics.clone();
        let group = Arc::new(Self {
            update_interval,
            remote_metrics,
            internal_metrics,
            inner: ArcSwap::new(Arc::new(Collection::new(im_clone))),
            admin_lock: Default::default(),
        });
        group.spawn_metrics_reporter();
        group
    }

    fn update_metrics_now(&self) {
        let metrics = self.metrics();
        self.remote_metrics
            .store(Arc::new((Instant::now(), metrics)));
    }

    fn spawn_metrics_reporter(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let update_interval = self.update_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(update_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // `interval` ticks immediately the first time. The topology paths
            // already publish an initial snapshot, so wait a full interval.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let Some(group) = weak.upgrade() else {
                    break;
                };
                group.update_metrics_now();
            }
        });
    }

    fn metrics(&self) -> internal_metrics::Channel {
        let mut channels: HashMap<String, ChannelState> = HashMap::default();
        {
            let guard = self.inner.load();
            for chan in guard.channels.iter() {
                channels.insert(chan.topic().to_string(), chan.state());
            }
        }
        self.internal_metrics.get(channels)
    }

    async fn broadcast(&self, payload: M) -> Result<(), Error<M>> {
        if payload.is_empty() {
            return Ok(());
        }
        let inner = self.inner.load();
        inner.broadcast(payload).await
    }

    #[cfg(test)]
    fn send_to(&self, topic: &T, payload: M) -> Result<(), Error<M>> {
        self.send_to_until(topic, payload, None)
    }

    fn send_to_until(
        &self,
        topic: &T,
        payload: M,
        stop: Option<&CancellationToken>,
    ) -> Result<(), Error<M>> {
        if payload.is_empty() {
            return Ok(());
        }
        let inner = self.inner.load();
        inner.send_to(topic, payload, stop)
    }

    async fn get_consumer(
        &self,
        topic: &T,
        conf: config::Channel,
    ) -> Result<Consumer<M>, Error<M>> {
        let consumer = {
            let _guard = self.admin_lock.lock().await;
            let inner = self.inner.load();
            if inner.get(topic).is_some() {
                return Err(Error::TopicAlreadyExists);
            }
            let mut collection = inner.as_ref().clone();
            let (chan, consumer) =
                Channel::new(topic.clone(), self.internal_metrics.clone(), conf, false).await?;
            collection.add(chan)?;
            drop(inner);
            self.inner.store(Arc::new(collection));
            consumer
        };
        self.internal_metrics.add_messages_sent(topic, 0);
        self.internal_metrics.add_messages_dropped(topic, 0);
        self.internal_metrics.add_batches_sent(topic, 0);
        self.internal_metrics.add_batches_dropped(topic, 0);
        self.update_metrics_now();
        Ok(consumer)
    }

    async fn replace(&self, topic: &T, conf: config::Channel) -> Result<Consumer<M>, Error<M>> {
        let _guard = self.admin_lock.lock().await;
        let inner = self.inner.load();
        let Some(old) = inner.get(topic) else {
            return Err(Error::TopicNotFound(None));
        };
        let left_behind = match (old, &conf) {
            (Channel::Disk { chan, .. }, config::Channel::Memory(_)) => Some(chan.writer.clone()),
            _ => None,
        };
        let mut collection = inner.as_ref().clone();
        let (chan, consumer) =
            Channel::new(topic.clone(), self.internal_metrics.clone(), conf, true).await?;
        if let Some(writer) = left_behind {
            // Before the old workers are cancelled, so what they hand back
            // reaches the new topic instead of the keyspace.
            writer.left_behind.store(true, Ordering::Relaxed);
            if !writer.keyspace.is_empty().unwrap_or(true) {
                // Nothing reads the keyspace until the topic is a disk one again.
                warn!(
                    topic = %topic,
                    bytes = writer.held(),
                    "disk backlog left behind by a switch to a memory channel"
                );
            }
        }
        // In one store: producers go from the old topic to the new one
        // without seeing it missing.
        collection.remove(topic, false)?;
        collection.add(chan)?;
        drop(inner);
        self.inner.store(Arc::new(collection));
        self.update_metrics_now();
        Ok(consumer)
    }

    async fn get_receiver(
        &self,
        topic: &T,
        conf: config::Channel,
    ) -> Result<flume::Receiver<M>, Error<M>> {
        if conf.write_through() {
            return Err(Error::Internal(anyhow::anyhow!(
                "write_through needs a consumer that acks"
            )));
        }
        let consumer = self.get_consumer(topic, conf).await?;
        Ok(consumer
            .into_receiver()
            .expect("only a write-through topic has a leased input"))
    }

    async fn remove(&self, topic: &T) -> Result<(), Error<M>> {
        {
            let _guard = self.admin_lock.lock().await;
            let inner = self.inner.load();
            if inner.get(topic).is_none() {
                return Err(Error::TopicNotFound(None));
            }
            let mut collection = inner.as_ref().clone();
            collection.remove(topic, true)?;
            drop(inner);
            self.inner.store(Arc::new(collection));
        }
        self.internal_metrics.remove_channel(topic);
        self.update_metrics_now();
        Ok(())
    }

    // this also removes all disk data, regular shutdown shouldn't use this
    async fn remove_all(&self) -> Result<(), Error<M>> {
        let _guard = self.admin_lock.lock().await;
        self.internal_metrics.remove_all_channels();
        let mut old = self
            .inner
            .swap(Arc::new(Collection::new(self.internal_metrics.clone())))
            .as_ref()
            .clone();
        self.update_metrics_now();
        old.remove_and_clear_all()
    }

    async fn close_all(&self) {
        let _guard = self.admin_lock.lock().await;
        self.internal_metrics.remove_all_channels();
        let old = self
            .inner
            .swap(Arc::new(Collection::new(self.internal_metrics.clone())));
        for chan in &old.channels {
            chan.close();
        }
        self.update_metrics_now();
    }
}

pub struct AdminChannel<T, M> {
    inner: Arc<ChannelGroup<T, M>>,
}

impl<T: Topic, M: Message> Clone for AdminChannel<T, M> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Topic, M: Message> AdminChannel<T, M> {
    pub async fn get_receiver(
        &self,
        topic: &T,
        conf: config::Channel,
    ) -> Result<flume::Receiver<M>, Error<M>> {
        self.inner.get_receiver(topic, conf).await
    }
    /// Like [`get_receiver`](Self::get_receiver), for a consumer that settles
    /// what it takes: requeues it on [`Consumer::nack`].
    pub async fn get_consumer(
        &self,
        topic: &T,
        conf: config::Channel,
    ) -> Result<Consumer<M>, Error<M>> {
        self.inner.get_consumer(topic, conf).await
    }
    /// Swaps the topic for a new one built from `conf`, keeping its metrics
    /// and, for a disk topic, its data. Producers move to the new topic at
    /// once. The old consumer gets what the old topic still holds and then
    /// sees its input end; hand that over with [`Consumer::adopt`].
    pub async fn replace(&self, topic: &T, conf: config::Channel) -> Result<Consumer<M>, Error<M>> {
        self.inner.replace(topic, conf).await
    }
    /// Drops the topic and, for a disk topic, deletes its data.
    pub async fn remove(&self, topic: &T) -> Result<(), Error<M>> {
        self.inner.remove(topic).await
    }
    /// [`remove`](Self::remove) for every topic.
    pub async fn remove_all(&self) -> Result<(), Error<M>> {
        self.inner.remove_all().await
    }
    /// Stops every topic taking messages: their receivers get what is
    /// queued, then see the channel disconnected. Unlike
    /// [`remove_all`](Self::remove_all), disk topics keep what is on disk for
    /// the next start.
    pub async fn close_all(&self) {
        self.inner.close_all().await
    }
}

pub struct SenderChannel<T, M> {
    inner: Arc<ChannelGroup<T, M>>,
}

impl<T: Topic, M: Message> Clone for SenderChannel<T, M> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: Topic, M: Message> SenderChannel<T, M> {
    pub async fn broadcast(&self, payload: M) -> Result<(), Error<M>> {
        self.inner.broadcast(payload).await
    }

    /// Blocks the calling thread while the topic is full, unless it is
    /// `non_blocking`.
    pub fn send_to(&self, topic: &T, payload: M) -> Result<(), Error<M>> {
        self.inner.send_to_until(topic, payload, None)
    }

    /// [`send_to`](Self::send_to) that waits on a full topic only until
    /// `stop` is cancelled. From then on a full memory topic fails the send
    /// with [`Error::BufferFull`], and a full disk topic takes the payload
    /// past its `max_disk_size`, so a consumer that stopped consuming cannot
    /// hold the caller.
    pub fn send_to_until(
        &self,
        topic: &T,
        payload: M,
        stop: &CancellationToken,
    ) -> Result<(), Error<M>> {
        self.inner.send_to_until(topic, payload, Some(stop))
    }
}

pub fn new<T: Topic, M: Message>(
    update_interval: Duration,
    remote_metrics: Arc<ArcSwap<(Instant, internal_metrics::Channel)>>,
) -> (AdminChannel<T, M>, SenderChannel<T, M>) {
    let inner = ChannelGroup::<T, M>::new(update_interval, remote_metrics);
    (
        AdminChannel {
            inner: inner.clone(),
        },
        SenderChannel { inner },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chan::Delivery;
    use crate::config;
    use crate::payload::Payload;
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use std::sync::Once;
    use tokio::time::timeout;

    type Remote = Arc<ArcSwap<(Instant, internal_metrics::Channel)>>;

    fn make_group_with_interval(
        update_interval: Duration,
    ) -> (Arc<ChannelGroup<String, Arc<Payload>>>, Remote) {
        let remote = Arc::new(ArcSwap::from_pointee((
            Instant::now(),
            internal_metrics::Channel::default(),
        )));
        (
            ChannelGroup::<String, Arc<Payload>>::new(update_interval, remote.clone()),
            remote,
        )
    }

    fn make_group() -> Arc<ChannelGroup<String, Arc<Payload>>> {
        make_group_with_interval(Duration::from_secs(60)).0
    }

    fn make_group_of<M: Message>() -> Arc<ChannelGroup<String, M>> {
        let remote = Arc::new(ArcSwap::from_pointee((
            Instant::now(),
            internal_metrics::Channel::default(),
        )));
        ChannelGroup::new(Duration::from_secs(60), remote)
    }

    /// A message whose disk encoding fails on demand: `encodes` false fails
    /// the write, `decodes` false writes a value that fails the read.
    #[derive(Clone, Debug)]
    struct Flaky {
        len: usize,
        encodes: bool,
        decodes: bool,
    }

    fn flaky(len: usize) -> Flaky {
        Flaky {
            len,
            encodes: true,
            decodes: true,
        }
    }

    impl HasLength for Flaky {
        fn len(&self) -> usize {
            self.len
        }
    }

    impl Codec for Flaky {
        fn encode(&self, out: &mut dyn std::io::Write, _: bool) -> Result<()> {
            anyhow::ensure!(self.encodes, "this message does not encode");
            out.write_all(&(self.len as u64).to_le_bytes())?;
            out.write_all(&u64::from(self.decodes).to_le_bytes())?;
            Ok(())
        }

        fn decode(buf: arrow::buffer::Buffer, _: bool) -> Result<Self> {
            let buf = buf.as_slice();
            anyhow::ensure!(buf[8] == 1, "this message does not decode");
            Ok(flaky(u64::from_le_bytes(buf[..8].try_into()?) as usize))
        }
    }

    /// Per-topic `batches_lost`.
    fn lost<M: Message>(group: &ChannelGroup<String, M>, topic: &str) -> usize {
        group
            .metrics()
            .batches_lost
            .get(topic)
            .copied()
            .unwrap_or_default()
    }

    async fn wait_for_lost<M: Message>(group: &ChannelGroup<String, M>, topic: &str, n: usize) {
        timeout(Duration::from_secs(10), async {
            while lost(group, topic) < n {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{topic} never lost {n}"));
    }

    /// The topic's keyspace, as the topic itself opens it.
    fn keyspace_of(topic: &str) -> Keyspace {
        DATABASE
            .get()
            .unwrap()
            .keyspace(topic, KeyspaceCreateOptions::default)
            .unwrap()
    }

    /// Waits until no bridge holds the keyspace's cursors, so the next open
    /// reads the keyspace afresh instead of sharing them.
    async fn wait_until_closed(topic: &str) {
        timeout(Duration::from_secs(10), async {
            loop {
                let open = OPEN_KEYSPACES
                    .lock()
                    .get(topic)
                    .is_some_and(|(write, _, _)| write.strong_count() > 0);
                if !open {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{topic} stayed open"));
    }

    // Build a logs payload carrying `n` records. The channel tracks
    // `HasLength::len` → `Payload::num_items`, which for logs is the record
    // count. `n == 0` yields an empty payload (no Logs batch is set).
    fn batch(n: usize) -> Arc<Payload> {
        let req = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: (0..n).map(|_| LogRecord::default()).collect(),
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        let data = encode_logs_otap_batch(&req).unwrap();
        Arc::new(Payload::new(data))
    }

    fn memory_conf(buffer: usize, non_blocking: bool) -> config::Channel {
        config::Channel::Memory(config::MemoryChannel {
            non_blocking,
            channel_buffer_size: buffer,
        })
    }

    fn disk_conf() -> config::Channel {
        config::Channel::Disk(config::DiskChannel {
            non_blocking: false,
            max_memtable_size: 8 * 1024 * 1024,
            max_disk_size: 0,
            worker_count: 2,
            compression: config::OtapCompression::None,
            transport_optimized: false,
            channel_buffer_size: 0,
            write_through: false,
        })
    }

    fn write_through_conf(buffer: usize) -> config::Channel {
        config::Channel::Disk(config::DiskChannel {
            channel_buffer_size: buffer,
            write_through: true,
            ..match disk_conf() {
                config::Channel::Disk(conf) => conf,
                config::Channel::Memory(_) => unreachable!(),
            }
        })
    }

    fn unique_topic(prefix: &str) -> String {
        format!("{prefix}-{}", uuid::Uuid::now_v7().simple())
    }

    fn ensure_db() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let dir = std::env::temp_dir().join(format!(
                "patrolli-chan-unit-{}",
                uuid::Uuid::now_v7().simple()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            init_database(&crate::config::global::DiskChannel {
                data_dir: dir.to_string_lossy().into_owned(),
                max_journal_size: 0,
                cache_size: 0,
            })
            .unwrap();
        });
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn memory_register_send_receive() {
        let group = make_group();
        let topic = "t1".to_string();
        let receiver = group
            .get_receiver(&topic, memory_conf(4, false))
            .await
            .unwrap();
        group.send_to(&topic, batch(3)).unwrap();
        let received = receiver.recv_async().await.unwrap();
        assert_eq!(received.len(), 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn duplicate_registration_fails() {
        let group = make_group();
        let topic = "t1".to_string();
        group
            .get_receiver(&topic, memory_conf(4, false))
            .await
            .unwrap();
        let err = group
            .get_receiver(&topic, memory_conf(4, false))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::TopicAlreadyExists));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_topic_then_send_fails() {
        let group = make_group();
        let topic = "t1".to_string();
        group
            .get_receiver(&topic, memory_conf(4, false))
            .await
            .unwrap();
        group.remove(&topic).await.unwrap();
        let err = group.send_to(&topic, batch(1)).unwrap_err();
        assert!(matches!(err, Error::TopicNotFound(_)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_unknown_topic_errors() {
        let group = make_group();
        let err = group.remove(&"unknown".to_string()).await.unwrap_err();
        assert!(matches!(err, Error::TopicNotFound(_)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_all_clears_topics() {
        let group = make_group();
        group
            .get_receiver(&"a".to_string(), memory_conf(4, false))
            .await
            .unwrap();
        group
            .get_receiver(&"b".to_string(), memory_conf(4, false))
            .await
            .unwrap();
        group.remove_all().await.unwrap();
        let err = group.send_to(&"a".to_string(), batch(1)).unwrap_err();
        assert!(matches!(err, Error::TopicNotFound(_)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn close_drains_a_memory_topic_then_disconnects() {
        let group = make_group();
        let topic = "t1".to_string();
        let receiver = group
            .get_receiver(&topic, memory_conf(4, false))
            .await
            .unwrap();
        for _ in 0..3 {
            group.send_to(&topic, batch(1)).unwrap();
        }
        group.close_all().await;
        assert!(matches!(
            group.send_to(&topic, batch(1)).unwrap_err(),
            Error::TopicNotFound(_)
        ));
        let mut received = 0;
        while receiver.recv_async().await.is_ok() {
            received += 1;
        }
        assert_eq!(received, 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn close_all_disconnects_every_topic() {
        let group = make_group();
        let a = group
            .get_receiver(&"a".to_string(), memory_conf(4, false))
            .await
            .unwrap();
        let b = group
            .get_receiver(&"b".to_string(), memory_conf(4, false))
            .await
            .unwrap();
        group.send_to(&"a".to_string(), batch(1)).unwrap();
        group.close_all().await;
        assert!(a.recv_async().await.is_ok());
        assert!(a.recv_async().await.is_err());
        assert!(b.recv_async().await.is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn close_keeps_a_disk_topic_for_the_next_start() {
        const SENT: usize = 20;
        ensure_db();
        let group = make_group();
        let topic = unique_topic("close-keep");
        let receiver = group.get_receiver(&topic, disk_conf()).await.unwrap();
        // Nobody reads, so the bridge persists what it is handed. `len`
        // tells the payloads apart.
        for n in 1..=SENT {
            group.broadcast(batch(n)).await.unwrap();
        }
        group.close_all().await;
        // The bridge hands over what it already took off disk, then stops.
        let mut lens = Vec::new();
        while let Ok(payload) = timeout(Duration::from_secs(10), receiver.recv_async())
            .await
            .expect("disk topic did not disconnect after close")
        {
            lens.push(payload.len());
        }
        assert!(lens.len() < SENT, "the backlog was drained instead of kept");
        let receiver = group.get_receiver(&topic, disk_conf()).await.unwrap();
        while lens.len() < SENT {
            let payload = timeout(Duration::from_secs(10), receiver.recv_async())
                .await
                .expect("backlog missing after reopening")
                .unwrap();
            lens.push(payload.len());
        }
        // Each exactly once, across both runs.
        assert!(
            timeout(Duration::from_millis(300), receiver.recv_async())
                .await
                .is_err(),
            "delivered more than once"
        );
        lens.sort();
        assert_eq!(lens, (1..=SENT).collect::<Vec<_>>());
        group.remove(&topic).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broadcast_delivers_to_all_topics() {
        let group = make_group();
        let recv_a = group
            .get_receiver(&"a".to_string(), memory_conf(4, false))
            .await
            .unwrap();
        let recv_b = group
            .get_receiver(&"b".to_string(), memory_conf(4, false))
            .await
            .unwrap();
        group.broadcast(batch(2)).await.unwrap();
        let a = recv_a.recv_async().await.unwrap();
        let b = recv_b.recv_async().await.unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 2);
    }

    /// Nothing took it, so the caller must not count it sent (#39).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broadcast_with_no_topics_hands_the_payload_back() {
        let group = make_group();
        match group.broadcast(batch(2)).await {
            Err(Error::TopicNotFound(Some(payload))) => assert_eq!(payload.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    /// A metrics payload holding one metric row that carries no data points.
    /// `num_items()` is 0 but there is a record, so the channel must deliver it.
    fn metrics_batch_without_data_points() -> Arc<Payload> {
        use otel_arrow_dfe_pdata::encode::encode_metrics_otap_batch;
        use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::{
            Gauge, Metric, MetricsData, ResourceMetrics, ScopeMetrics, metric,
        };
        let req = MetricsData {
            resource_metrics: vec![ResourceMetrics {
                resource: None,
                scope_metrics: vec![ScopeMetrics {
                    scope: None,
                    metrics: vec![Metric {
                        name: "m".to_string(),
                        description: String::new(),
                        unit: String::new(),
                        metadata: vec![],
                        data: Some(metric::Data::Gauge(Gauge {
                            data_points: vec![],
                        })),
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        Arc::new(Payload::new(encode_metrics_otap_batch(&req).unwrap()))
    }

    /// Emptiness is decided on records, not items — otherwise a metrics payload
    /// whose rows have no data points is silently dropped at the channel.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broadcast_delivers_metrics_without_data_points() {
        let pl = metrics_batch_without_data_points();
        assert_eq!(pl.num_items(), 0);
        assert_eq!(pl.num_rows(), 1);

        let group = make_group();
        let recv = group
            .get_receiver(&"a".to_string(), memory_conf(4, false))
            .await
            .unwrap();
        group.broadcast(pl).await.unwrap();
        let got = timeout(Duration::from_millis(500), recv.recv_async())
            .await
            .expect("must not be dropped as empty")
            .expect("recv");
        assert_eq!(got.num_rows(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broadcast_empty_batch_is_noop() {
        let group = make_group();
        let recv = group
            .get_receiver(&"a".to_string(), memory_conf(4, false))
            .await
            .unwrap();
        group.broadcast(batch(0)).await.unwrap();
        let res = timeout(Duration::from_millis(50), recv.recv_async()).await;
        assert!(
            res.is_err(),
            "broadcast of empty batch should not deliver anything"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_to_empty_batch_is_noop() {
        let group = make_group();
        let topic = "t".to_string();
        let _recv = group
            .get_receiver(&topic, memory_conf(4, false))
            .await
            .unwrap();
        group.send_to(&topic, batch(0)).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn memory_non_blocking_returns_buffer_full() {
        let group = make_group();
        let topic = "t1".to_string();
        let _recv = group
            .get_receiver(&topic, memory_conf(1, true))
            .await
            .unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        let err = group.send_to(&topic, batch(1)).unwrap_err();
        assert!(matches!(err, Error::BufferFull(_)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn internal_metrics_track_sends() {
        let group = make_group();
        let topic = "t1".to_string();
        let _recv = group
            .get_receiver(&topic, memory_conf(8, false))
            .await
            .unwrap();
        group.send_to(&topic, batch(5)).unwrap();
        group.send_to(&topic, batch(3)).unwrap();
        let total_messages = group
            .internal_metrics
            .total_messages
            .load(Ordering::Relaxed);
        let total_batches = group.internal_metrics.total_batches.load(Ordering::Relaxed);
        assert_eq!(total_messages, 8);
        assert_eq!(total_batches, 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn periodic_snapshot_refreshes_memory_queue_state() {
        let (group, remote) = make_group_with_interval(Duration::from_millis(20));
        let topic = "periodic".to_string();
        let receiver = group
            .get_receiver(&topic, memory_conf(8, false))
            .await
            .unwrap();

        group.send_to(&topic, batch(1)).unwrap();
        group.update_metrics_now();
        let populated_at = remote.load().0;
        assert_eq!(remote.load().1.channels[&topic].kind, ChannelKind::Memory);
        assert_eq!(remote.load().1.channels[&topic].length, 1);

        receiver.recv_async().await.unwrap();
        timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = remote.load();
                if snapshot.0 > populated_at && snapshot.1.channels[&topic].length == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("periodic reporter did not refresh the drained queue");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn disk_register_send_receive() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("disk-rsr");
        let receiver = group.get_receiver(&topic, disk_conf()).await.unwrap();
        let snapshot = group.metrics();
        assert_eq!(snapshot.channels[&topic].kind, ChannelKind::Disk);
        group.send_to(&topic, batch(4)).unwrap();
        let received = timeout(Duration::from_secs(5), receiver.recv_async())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.len(), 4);
        group.remove(&topic).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn disk_persists_when_consumer_is_slow() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("disk-persist");
        let receiver = group.get_receiver(&topic, disk_conf()).await.unwrap();

        const N: usize = 20;
        for _ in 0..N {
            group.send_to(&topic, batch(2)).unwrap();
        }

        let mut total_records = 0;
        for _ in 0..N {
            let b = timeout(Duration::from_secs(5), receiver.recv_async())
                .await
                .unwrap()
                .unwrap();
            total_records += b.len();
        }
        assert_eq!(total_records, N * 2);
        group.remove(&topic).await.unwrap();
    }

    async fn recv_one<M: Message>(consumer: &Consumer<M>) -> Option<Delivery<M>> {
        timeout(Duration::from_secs(10), consumer.recv())
            .await
            .expect("recv timed out")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nack_wakes_a_worker_parked_in_recv() {
        let group = make_group();
        let topic = "t1".to_string();
        let consumer = group
            .get_consumer(&topic, memory_conf(4, false))
            .await
            .unwrap();
        group.send_to(&topic, batch(3)).unwrap();
        let delivery = recv_one(&consumer).await.unwrap();
        let parked = tokio::spawn({
            let consumer = consumer.clone();
            async move { consumer.recv().await.map(|d| d.msg.len()) }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!parked.is_finished());
        consumer.nack(delivery).await;
        let got = timeout(Duration::from_secs(5), parked)
            .await
            .expect("the nack did not wake the parked worker")
            .unwrap();
        assert_eq!(got, Some(3));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nack_on_a_full_channel_is_stashed_and_received_first() {
        let group = make_group();
        let topic = "t1".to_string();
        let consumer = group
            .get_consumer(&topic, memory_conf(1, false))
            .await
            .unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        let first = recv_one(&consumer).await.unwrap();
        group.send_to(&topic, batch(2)).unwrap();
        consumer.nack(first).await;
        assert_eq!(consumer.stashed(), 1);
        assert_eq!(recv_one(&consumer).await.unwrap().msg.len(), 1);
        assert_eq!(consumer.stashed(), 0);
        assert_eq!(recv_one(&consumer).await.unwrap().msg.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recv_drains_the_stash_after_the_input_disconnects() {
        let group = make_group();
        let topic = "t1".to_string();
        let consumer = group
            .get_consumer(&topic, memory_conf(4, false))
            .await
            .unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        group.send_to(&topic, batch(2)).unwrap();
        let first = recv_one(&consumer).await.unwrap();
        group.close_all().await;
        // No sender is left to requeue through, so it is stashed, and comes
        // back ahead of what the input still holds.
        consumer.nack(first).await;
        assert_eq!(consumer.stashed(), 1);
        assert_eq!(recv_one(&consumer).await.unwrap().msg.len(), 1);
        let second = recv_one(&consumer).await.unwrap();
        assert_eq!(second.msg.len(), 2);
        // Requeued once the input is drained as well as disconnected.
        consumer.nack(second).await;
        assert_eq!(recv_one(&consumer).await.unwrap().msg.len(), 2);
        assert!(recv_one(&consumer).await.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_held_consumer_does_not_keep_its_input_open() {
        ensure_db();
        let group = make_group();
        let memory = group
            .get_consumer(&"mem".to_string(), memory_conf(4, false))
            .await
            .unwrap();
        let disk_topic = unique_topic("consumer-open");
        let disk = group.get_consumer(&disk_topic, disk_conf()).await.unwrap();
        let (memory_copy, disk_copy) = (memory.clone(), disk.clone());
        group.close_all().await;
        assert!(recv_one(&memory).await.is_none());
        assert!(recv_one(&memory_copy).await.is_none());
        assert!(recv_one(&disk).await.is_none());
        assert!(recv_one(&disk_copy).await.is_none());
        let disk = group.get_consumer(&disk_topic, disk_conf()).await.unwrap();
        drop(disk);
        group.remove(&disk_topic).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_disk_topic_nack_goes_back_to_its_keyspace() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("consumer-nack");
        let consumer = group.get_consumer(&topic, disk_conf()).await.unwrap();
        group.send_to(&topic, batch(4)).unwrap();
        let delivery = recv_one(&consumer).await.unwrap();
        group.close_all().await;
        // The bridge is gone once the input ends, so whatever comes back
        // after the reopen can only have come from the keyspace.
        assert!(recv_one(&consumer).await.is_none());
        consumer.nack(delivery).await;
        assert_eq!(consumer.stashed(), 0);
        let reopened = group.get_consumer(&topic, disk_conf()).await.unwrap();
        assert_eq!(recv_one(&reopened).await.unwrap().msg.len(), 4);
        group.remove(&topic).await.unwrap();
    }

    /// Waits for the background deletes of acked keys to land.
    async fn wait_for_keys(topic: &str, expected: usize) {
        let keyspace = DATABASE
            .get()
            .unwrap()
            .keyspace(topic, KeyspaceCreateOptions::default)
            .unwrap();
        timeout(Duration::from_secs(10), async {
            while keyspace.len().unwrap() != expected {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{topic} never got to {expected} keys"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn write_through_redelivers_exactly_the_unacked_keys_after_a_restart() {
        const SENT: usize = 10;
        ensure_db();
        let group = make_group();
        let topic = unique_topic("wt-restart");
        let consumer = group
            .get_consumer(&topic, write_through_conf(4))
            .await
            .unwrap();
        // `len` tells the payloads apart.
        for n in 1..=SENT {
            group.send_to(&topic, batch(n)).unwrap();
        }
        let mut unacked = Vec::new();
        for _ in 0..SENT {
            let (msg, token) = recv_one(&consumer).await.unwrap().into_parts();
            if msg.len() % 2 == 0 {
                consumer.ack(token);
            } else {
                // Held by a worker that is then killed: dropped, never settled.
                drop(token);
                unacked.push(msg.len());
            }
        }
        wait_for_keys(&topic, unacked.len()).await;
        group.close_all().await;
        drop(consumer);

        let reopened = group
            .get_consumer(&topic, write_through_conf(4))
            .await
            .unwrap();
        let mut redelivered = Vec::new();
        for _ in 0..unacked.len() {
            let (msg, token) = recv_one(&reopened).await.unwrap().into_parts();
            redelivered.push(msg.len());
            reopened.ack(token);
        }
        // Two bridge senders race each other, so neither pass comes in
        // send order.
        unacked.sort();
        redelivered.sort();
        assert_eq!(redelivered, unacked);
        // Nothing acked comes back.
        assert!(
            timeout(Duration::from_millis(300), reopened.recv())
                .await
                .is_err()
        );
        wait_for_keys(&topic, 0).await;
        group.remove(&topic).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn write_through_is_on_disk_before_the_send_returns() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("wt-send");
        let consumer = group
            .get_consumer(&topic, write_through_conf(4))
            .await
            .unwrap();
        group.broadcast(batch(3)).await.unwrap();
        group.send_to(&topic, batch(2)).unwrap();
        // Read and still on disk: a read only leases the key.
        let first = recv_one(&consumer).await.unwrap();
        let second = recv_one(&consumer).await.unwrap();
        wait_for_keys(&topic, 2).await;
        consumer.ack_all([first, second].map(|d| d.into_parts().1));
        wait_for_keys(&topic, 0).await;
        group.remove(&topic).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_write_through_nack_does_not_duplicate_the_key() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("wt-nack");
        // No buffer: the requeue has no room in the input and is stashed.
        let consumer = group
            .get_consumer(&topic, write_through_conf(0))
            .await
            .unwrap();
        group.send_to(&topic, batch(5)).unwrap();
        let delivery = recv_one(&consumer).await.unwrap();
        consumer.nack(delivery).await;
        let delivery = recv_one(&consumer).await.unwrap();
        assert_eq!(delivery.msg.len(), 5);
        // Still the one key, held for the redelivery.
        wait_for_keys(&topic, 1).await;
        consumer.nack(delivery).await;
        group.close_all().await;
        // Settled after the topic closed: the stash still carries the key.
        let delivery = recv_one(&consumer).await.unwrap();
        consumer.ack(delivery.into_parts().1);
        assert!(recv_one(&consumer).await.is_none());
        wait_for_keys(&topic, 0).await;
        drop(consumer);

        let reopened = group
            .get_consumer(&topic, write_through_conf(0))
            .await
            .unwrap();
        assert!(
            timeout(Duration::from_millis(300), reopened.recv())
                .await
                .is_err()
        );
        group.remove(&topic).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_write_through_topic_needs_a_consumer() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("wt-receiver");
        assert!(matches!(
            group.get_receiver(&topic, write_through_conf(0)).await,
            Err(Error::Internal(_))
        ));
        // Refused before the topic was created.
        assert!(matches!(
            group.send_to(&topic, batch(1)),
            Err(Error::TopicNotFound(_))
        ));
    }

    /// Takes everything `old` still holds over to `new`, as a sink replacing
    /// its channel does.
    /// `conf` as a disk topic that is full once it holds a single byte, and
    /// says so instead of waiting.
    fn tiny(conf: config::Channel) -> config::Channel {
        match conf {
            config::Channel::Disk(conf) => config::Channel::Disk(config::DiskChannel {
                max_disk_size: 1,
                non_blocking: true,
                ..conf
            }),
            config::Channel::Memory(_) => unreachable!(),
        }
    }

    /// Flushes what `topic` deleted so far into a table: its files then
    /// hold the deleted payloads until a compaction, whatever is read off it.
    async fn flush(topic: &str) {
        let keyspace = DATABASE
            .get()
            .unwrap()
            .keyspace(topic, KeyspaceCreateOptions::default)
            .unwrap();
        tokio::task::spawn_blocking(move || keyspace.rotate_memtable_and_wait().unwrap())
            .await
            .unwrap();
    }

    /// Sends until the topic takes the payload: an ack's delete lands off the
    /// async thread.
    async fn send_eventually(group: &ChannelGroup<String, Arc<Payload>>, topic: &String) {
        timeout(Duration::from_secs(10), async {
            while let Err(e) = group.send_to(topic, batch(1)) {
                assert!(matches!(e, Error::BufferFull(_)), "{e}");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the drained topic never took a payload again");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_drained_disk_topic_takes_payloads_again() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("disk-full");
        let consumer = group.get_consumer(&topic, tiny(disk_conf())).await.unwrap();
        // The bridge pops what it can hand on and holds it, so a few go
        // through before one stays on disk.
        let mut sent = 0;
        timeout(Duration::from_secs(10), async {
            loop {
                match group.send_to(&topic, batch(1)) {
                    Ok(()) => sent += 1,
                    Err(Error::BufferFull(_)) => break,
                    Err(e) => panic!("{e}"),
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the topic never filled up");
        flush(&topic).await;
        for _ in 0..sent {
            consumer.ack(recv_one(&consumer).await.unwrap().into_parts().1);
        }
        send_eventually(&group, &topic).await;
        group.remove(&topic).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_drained_write_through_topic_takes_payloads_again() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("wt-full");
        let consumer = group
            .get_consumer(&topic, tiny(write_through_conf(4)))
            .await
            .unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        // Read and still held: a lease counts until it is acked.
        let (_, token) = recv_one(&consumer).await.unwrap().into_parts();
        assert!(matches!(
            group.send_to(&topic, batch(1)),
            Err(Error::BufferFull(_))
        ));
        flush(&topic).await;
        consumer.ack(token);
        send_eventually(&group, &topic).await;
        group.remove(&topic).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_reopened_topic_counts_what_the_last_run_left() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("wt-full-reopen");
        let consumer = group
            .get_consumer(&topic, write_through_conf(4))
            .await
            .unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        wait_for_keys(&topic, 1).await;
        group.close_all().await;
        drop(consumer);

        let reopened = group
            .get_consumer(&topic, tiny(write_through_conf(4)))
            .await
            .unwrap();
        assert!(matches!(
            group.send_to(&topic, batch(1)),
            Err(Error::BufferFull(_))
        ));
        reopened.ack(recv_one(&reopened).await.unwrap().into_parts().1);
        send_eventually(&group, &topic).await;
        group.remove(&topic).await.unwrap();
    }

    /// Files under a keyspace's `dir` (`tables`, `blobs`).
    fn files_in(keyspace: &Keyspace, dir: &str) -> usize {
        std::fs::read_dir(keyspace.path().join(dir))
            .map(|entries| entries.count())
            .unwrap_or(0)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_drained_topic_gets_its_files_back() {
        const SENT: usize = 20;
        ensure_db();
        let group = make_group();
        let topic = unique_topic("disk-compact");
        let consumer = group
            .get_consumer(&topic, write_through_conf(SENT))
            .await
            .unwrap();
        // Big enough for fjall to keep each in a blob file, not inline.
        for _ in 0..SENT {
            group.send_to(&topic, batch(100)).unwrap();
        }
        wait_for_keys(&topic, SENT).await;
        flush(&topic).await;
        let keyspace = DATABASE
            .get()
            .unwrap()
            .keyspace(&topic, KeyspaceCreateOptions::default)
            .unwrap();
        assert!(
            files_in(&keyspace, "blobs") > 0,
            "nothing went to a blob file"
        );
        for _ in 0..SENT {
            consumer.ack(recv_one(&consumer).await.unwrap().into_parts().1);
        }
        wait_for_keys(&topic, 0).await;
        // Nothing is written from here on, which on its own left the files
        // in place for good.
        timeout(Duration::from_secs(10), async {
            while keyspace.disk_space() > 0 || files_in(&keyspace, "blobs") > 0 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the delivered payloads' files were never deleted");
        group.remove(&topic).await.unwrap();
    }

    async fn adopt_all(old: &Consumer<Arc<Payload>>, new: &Consumer<Arc<Payload>>) -> usize {
        let mut moved = 0;
        while let Some(delivery) = recv_one(old).await {
            new.adopt(old, delivery).await;
            moved += 1;
        }
        moved
    }

    /// Receives `n` payloads, acking each, and returns their lengths sorted;
    /// then checks nothing else comes.
    async fn recv_lens<M: Message>(consumer: &Consumer<M>, n: usize) -> Vec<usize> {
        let mut lens = Vec::new();
        for _ in 0..n {
            let (msg, token) = recv_one(consumer).await.unwrap().into_parts();
            lens.push(msg.len());
            consumer.ack(token);
        }
        assert!(
            timeout(Duration::from_millis(300), consumer.recv())
                .await
                .is_err(),
            "delivered more than once"
        );
        lens.sort();
        lens
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn replace_moves_producers_and_the_old_queue_to_the_new_topic() {
        let group = make_group();
        let topic = "t1".to_string();
        let old = group
            .get_consumer(&topic, memory_conf(8, false))
            .await
            .unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        group.send_to(&topic, batch(2)).unwrap();
        let held = recv_one(&old).await.unwrap();
        let new = group.replace(&topic, memory_conf(4, false)).await.unwrap();
        group.send_to(&topic, batch(3)).unwrap();
        // A worker of the old topic hands back what it held as it goes.
        old.nack(held).await;
        assert_eq!(adopt_all(&old, &new).await, 2);
        assert_eq!(recv_lens(&new, 3).await, vec![1, 2, 3]);
        assert!(matches!(
            group
                .replace(&"missing".to_string(), memory_conf(1, false))
                .await,
            Err(Error::TopicNotFound(_))
        ));
    }

    /// The backlog stays on disk and the new topic reads it, across a change
    /// of compression, of `transport_optimized` and of write-through, without
    /// a key read twice.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn replacing_a_disk_topic_keeps_its_backlog() {
        const SENT: usize = 20;
        ensure_db();
        let group = make_group();
        let topic = unique_topic("replace-disk");
        let old = group.get_consumer(&topic, disk_conf()).await.unwrap();
        // Nobody reads, so the bridge persists what it is handed.
        for n in 1..=SENT {
            group.broadcast(batch(n)).await.unwrap();
        }
        let held = recv_one(&old).await.unwrap();
        let mut conf = write_through_conf(4);
        if let config::Channel::Disk(disk) = &mut conf {
            disk.compression = config::OtapCompression::Lz4;
            disk.transport_optimized = true;
        }
        let new = group.replace(&topic, conf).await.unwrap();
        group.broadcast(batch(SENT + 1)).await.unwrap();
        old.nack(held).await;
        adopt_all(&old, &new).await;
        assert_eq!(
            recv_lens(&new, SENT + 1).await,
            (1..=SENT + 1).collect::<Vec<_>>()
        );
        wait_for_keys(&topic, 0).await;
        group.remove(&topic).await.unwrap();
    }

    /// Write-through to the fast path: a lease the old consumer held is
    /// written as a new key and its old key deleted, so it is neither lost
    /// nor left behind to come back on the next start.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn replacing_a_write_through_topic_settles_its_leases() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("replace-wt");
        let old = group
            .get_consumer(&topic, write_through_conf(0))
            .await
            .unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        group.send_to(&topic, batch(2)).unwrap();
        let held = recv_one(&old).await.unwrap();
        let new = group.replace(&topic, disk_conf()).await.unwrap();
        old.nack(held).await;
        adopt_all(&old, &new).await;
        assert_eq!(recv_lens(&new, 2).await, vec![1, 2]);
        wait_for_keys(&topic, 0).await;
        group.remove(&topic).await.unwrap();
    }

    /// Disk to memory: the keyspace is left behind unread, so what the old
    /// consumer hands back after the switch must reach the new topic, not
    /// the keyspace.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn replacing_a_disk_topic_with_memory_moves_what_its_consumer_held() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("replace-to-memory");
        let old = group.get_consumer(&topic, disk_conf()).await.unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        let held = recv_one(&old).await.unwrap();
        let new = group.replace(&topic, memory_conf(4, false)).await.unwrap();
        old.nack(held).await;
        assert_eq!(adopt_all(&old, &new).await, 1);
        assert_eq!(recv_lens(&new, 1).await, vec![1]);
        wait_for_keys(&topic, 0).await;
    }

    /// Only the keyspaces picked go, whatever they hold; the rest keep their
    /// backlog.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_keyspaces_deletes_only_what_it_picks() {
        ensure_db();
        let kept = unique_topic("sweep-kept");
        let gone = unique_topic("sweep-gone");
        for topic in [&kept, &gone] {
            let group = make_group();
            let consumer = group.get_consumer(topic, disk_conf()).await.unwrap();
            for n in 1..=3 {
                group.send_to(topic, batch(n)).unwrap();
            }
            // Closed, not removed: the backlog stays, as at shutdown.
            group.close_all().await;
            drop(consumer);
        }
        wait_for_keys(&kept, 3).await;

        let target = gone.clone();
        let removed = remove_keyspaces(move |name| name == target).await.unwrap();
        assert_eq!(removed, vec![gone.clone()]);
        let names = DATABASE.get().unwrap().list_keyspace_names();
        assert!(!names.iter().any(|name| &**name == gone.as_str()));
        wait_for_keys(&kept, 3).await;
        remove_keyspaces(move |name| name == kept).await.unwrap();
    }

    /// Writers race each other while the reader pops right behind them: a
    /// key that became visible before a lower one would let the read cursor
    /// pass the lower one, which is then never popped. Drives the bridge
    /// directly, since through a channel a waiting consumer takes most
    /// payloads before they reach the disk.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn disk_pops_every_key_while_writers_race_the_reader() {
        const WRITERS: usize = 8;
        const PER_WRITER: usize = 500;
        ensure_db();
        let db = DATABASE.get().unwrap();
        let keyspace = db
            .keyspace(&unique_topic("disk-race"), KeyspaceCreateOptions::default)
            .unwrap();
        let (sender, _) = flume::bounded(0);
        let (_, receiver) = flume::bounded(0);
        let conf = config::DiskChannel {
            compression: config::OtapCompression::None,
            ..Default::default()
        };
        let bridge = Arc::new(DiskBridge::<String, Arc<Payload>> {
            topic: "disk-race".to_string(),
            conf,
            keyspace: keyspace.clone(),
            writer: DiskWriter::new(
                keyspace.clone(),
                &conf,
                Cursors::new(1, 0),
                Arc::new(AtomicBool::new(false)),
            ),
            delete_on_exit: Arc::new(AtomicBool::new(false)),
            worker_count: Arc::new(AtomicUsize::new(0)),
            sender: BridgeOutput::Plain(sender),
            receiver,
            metrics: Arc::default(),
        });

        let writers: Vec<_> = (0..WRITERS)
            .map(|_| {
                let bridge = bridge.clone();
                tokio::spawn(async move {
                    // Paced so the reader keeps up: it only skips a key
                    // while it is at the writers' frontier.
                    for _ in 0..PER_WRITER {
                        bridge.dump_to_disk(&batch(1)).await.unwrap();
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
            })
            .collect();
        let mut popped = 0;
        while writers.iter().any(|w| !w.is_finished()) {
            if bridge.get_from_disk().await.unwrap().is_some() {
                popped += 1;
            } else {
                tokio::task::yield_now().await;
            }
        }
        while bridge.get_from_disk().await.unwrap().is_some() {
            popped += 1;
        }
        assert_eq!(
            popped,
            WRITERS * PER_WRITER,
            "keys left behind the read cursor"
        );
        db.delete_keyspace(keyspace).unwrap();
    }

    /// A blocking memory topic nobody reads holds a sync send until `stop`,
    /// then turns it away, and from then on does not wait at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_to_until_stops_waiting_on_a_full_memory_topic() {
        let group = make_group();
        let topic = "stuck".to_string();
        let _rx = group
            .get_receiver(&topic, memory_conf(1, false))
            .await
            .unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        let sender = SenderChannel {
            inner: group.clone(),
        };
        let stop = CancellationToken::new();
        let send = {
            let (sender, topic, stop) = (sender.clone(), topic.clone(), stop.clone());
            std::thread::spawn(move || sender.send_to_until(&topic, batch(2), &stop))
        };
        std::thread::sleep(STOP_POLL * 3);
        assert!(!send.is_finished(), "a full topic did not hold the send");
        let stopped = Instant::now();
        stop.cancel();
        let res = send.join().unwrap();
        assert!(matches!(res, Err(Error::BufferFull(_))), "{res:?}");
        assert!(stopped.elapsed() < STOP_POLL * 2, "{:?}", stopped.elapsed());
        let started = Instant::now();
        let res = sender.send_to_until(&topic, batch(3), &stop);
        assert!(matches!(res, Err(Error::BufferFull(_))), "{res:?}");
        assert!(started.elapsed() < STOP_POLL, "{:?}", started.elapsed());
    }

    /// A blocking disk topic at its `max_disk_size` holds a sync send until
    /// `stop`, then takes it past the cap: nothing sent is lost.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_to_until_writes_past_a_full_disk_topic() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("disk-stop");
        let conf = match disk_conf() {
            config::Channel::Disk(conf) => config::Channel::Disk(config::DiskChannel {
                max_disk_size: 1,
                ..conf
            }),
            config::Channel::Memory(_) => unreachable!(),
        };
        let consumer = group.get_consumer(&topic, conf).await.unwrap();
        let sender = SenderChannel {
            inner: group.clone(),
        };
        let stop = CancellationToken::new();
        // The bridge holds one payload it cannot hand on and the next stays
        // on disk, so a few sends in the topic is full and holds the rest.
        let sends = 6;
        let send = {
            let (topic, stop) = (topic.clone(), stop.clone());
            std::thread::spawn(move || {
                (1..=sends)
                    .map(|n| sender.send_to_until(&topic, batch(n), &stop))
                    .collect::<Vec<_>>()
            })
        };
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!send.is_finished(), "a full topic did not hold the sends");
        stop.cancel();
        let results = tokio::task::spawn_blocking(move || send.join().unwrap())
            .await
            .unwrap();
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        let mut records = 0;
        for _ in 0..sends {
            let delivery = recv_one(&consumer).await.unwrap();
            let (payload, token) = delivery.into_parts();
            records += payload.len();
            consumer.ack(token);
        }
        assert_eq!(records, (1..=sends).sum::<usize>());
        group.remove(&topic).await.unwrap();
    }

    /// Every way a send fails counts as dropped on its topic, and only
    /// there: `total_batches` is always sent plus dropped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_sends_count_as_dropped_per_topic() {
        ensure_db();
        let group = make_group_of::<Flaky>();
        let (full, missing, gone) = (
            "full".to_string(),
            "missing".to_string(),
            "gone".to_string(),
        );
        let _full_rx = group
            .get_receiver(&full, memory_conf(1, true))
            .await
            .unwrap();
        group.send_to(&full, flaky(1)).unwrap();
        assert!(matches!(
            group.send_to(&full, flaky(2)),
            Err(Error::BufferFull(_))
        ));
        assert!(matches!(
            group.send_to(&missing, flaky(3)),
            Err(Error::TopicNotFound(Some(_)))
        ));
        drop(
            group
                .get_receiver(&gone, memory_conf(1, false))
                .await
                .unwrap(),
        );
        assert!(matches!(
            group.send_to(&gone, flaky(4)),
            Err(Error::Closed(_))
        ));
        let unwritable = unique_topic("wt-unwritable");
        let _consumer = group
            .get_consumer(&unwritable, write_through_conf(1))
            .await
            .unwrap();
        let payload = Flaky {
            encodes: false,
            ..flaky(5)
        };
        assert!(matches!(
            group.send_to(&unwritable, payload),
            Err(Error::Internal(_))
        ));

        let metrics = group.metrics();
        let get = |map: &HashMap<String, usize>, topic: &str| map.get(topic).copied().unwrap_or(0);
        for (topic, sent, dropped) in [
            (&full, 1, 2),
            (&missing, 0, 3),
            (&gone, 0, 4),
            (&unwritable, 0, 5),
        ] {
            assert_eq!(get(&metrics.messages_sent, topic), sent, "{topic}");
            assert_eq!(get(&metrics.batches_sent, topic), sent, "{topic}");
            assert_eq!(get(&metrics.messages_dropped, topic), dropped, "{topic}");
            assert_eq!(get(&metrics.batches_dropped, topic), 1, "{topic}");
        }
        assert_eq!(metrics.total_batches, 5);
        assert_eq!(metrics.total_messages, 15);
        group.remove(&unwritable).await.unwrap();
    }

    /// A value that does not decode is deleted, or it would stall the
    /// topic, and counted lost; what follows it is delivered (#3).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_undecodable_disk_value_is_skipped_and_counted() {
        ensure_db();
        for conf in [disk_conf(), write_through_conf(4)] {
            let group = make_group_of::<Flaky>();
            let topic = unique_topic("undecodable");
            let consumer = group.get_consumer(&topic, conf).await.unwrap();
            // Nobody reads, so even the plain topic writes it to disk.
            let payload = Flaky {
                decodes: false,
                ..flaky(1)
            };
            group.send_to(&topic, payload).unwrap();
            wait_for_lost(&group, &topic, 1).await;
            group.send_to(&topic, flaky(2)).unwrap();
            assert_eq!(recv_lens(&consumer, 1).await, vec![2]);
            assert_eq!(lost(&group, &topic), 1);
            wait_for_keys(&topic, 0).await;
            group.remove(&topic).await.unwrap();
        }
    }

    /// A key that is not 8 bytes is deleted and counted lost instead of
    /// stalling the read cursor in front of every payload behind it (N24).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_malformed_disk_key_does_not_stall_the_topic() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("bad-key");
        let consumer = group.get_consumer(&topic, disk_conf()).await.unwrap();
        // Sorts after the cursor's start and ahead of key 1.
        let mut bad = [0u8; 9];
        bad[8] = 5;
        keyspace_of(&topic).insert(bad, b"junk").unwrap();
        for n in 1..=3 {
            group.send_to(&topic, batch(n)).unwrap();
        }
        assert_eq!(recv_lens(&consumer, 3).await, vec![1, 2, 3]);
        assert_eq!(lost(&group, &topic), 1);
        wait_for_keys(&topic, 0).await;
        group.remove(&topic).await.unwrap();
    }

    /// A last key that does not parse used to clear the whole keyspace on
    /// open (#7). Now the backlog is kept, and writes go on after it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unparseable_last_key_keeps_the_backlog() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("bad-last-key");
        let consumer = group
            .get_consumer(&topic, write_through_conf(4))
            .await
            .unwrap();
        for n in 1..=3 {
            group.send_to(&topic, batch(n)).unwrap();
        }
        wait_for_keys(&topic, 3).await;
        group.close_all().await;
        drop(consumer);
        wait_until_closed(&topic).await;
        // Sorts after every key a topic writes.
        keyspace_of(&topic).insert([0xFF; 9], b"junk").unwrap();

        let reopened = group
            .get_consumer(&topic, write_through_conf(4))
            .await
            .unwrap();
        group.send_to(&topic, batch(4)).unwrap();
        assert_eq!(recv_lens(&reopened, 4).await, vec![1, 2, 3, 4]);
        wait_for_lost(&group, &topic, 1).await;
        wait_for_keys(&topic, 0).await;
        group.remove(&topic).await.unwrap();
    }

    /// With its consumer gone, a disk topic keeps taking what it is sent
    /// instead of panicking (N23), and what its bridge held is written back
    /// (#5): all of it comes back once the topic is open again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_disk_topic_whose_consumer_is_gone_keeps_what_it_is_sent() {
        const SENT: usize = 8;
        ensure_db();
        let group = make_group();
        let topic = unique_topic("consumer-gone");
        let consumer = group.get_consumer(&topic, disk_conf()).await.unwrap();
        // Nobody reads: the bridge's senders each hold one off disk.
        for n in 1..=3 {
            group.broadcast(batch(n)).await.unwrap();
        }
        drop(consumer);
        // More than the bridge has receivers, each of which used to die on
        // its first payload, leaving the next send waiting for good.
        for n in 4..=SENT {
            timeout(Duration::from_secs(5), group.broadcast(batch(n)))
                .await
                .expect("a send to a topic without a consumer hung")
                .unwrap();
        }
        group.close_all().await;
        wait_until_closed(&topic).await;
        let reopened = group.get_consumer(&topic, disk_conf()).await.unwrap();
        assert_eq!(
            recv_lens(&reopened, SENT).await,
            (1..=SENT).collect::<Vec<_>>()
        );
        assert_eq!(lost(&group, &topic), 0);
        group.remove(&topic).await.unwrap();
    }

    /// A send still on its way through the topic a replace took over lands
    /// in the keyspace both share, so the new topic delivers it (N23).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_send_on_its_way_through_a_replaced_disk_topic_reaches_the_new_one() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("replace-in-flight");
        let old = group.get_consumer(&topic, disk_conf()).await.unwrap();
        // What a producer loaded before the replace.
        let in_flight = group.inner.load_full();
        let new = group.replace(&topic, disk_conf()).await.unwrap();
        drop(old);
        for n in 1..=3 {
            timeout(Duration::from_secs(5), in_flight.broadcast(batch(n)))
                .await
                .expect("a send through the replaced topic hung")
                .unwrap();
        }
        assert_eq!(recv_lens(&new, 3).await, vec![1, 2, 3]);
        drop(in_flight);
        group.remove(&topic).await.unwrap();
    }

    /// A send still on its way to a removed topic is turned away: its
    /// keyspace is going away.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_send_on_its_way_to_a_removed_disk_topic_is_turned_away() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("remove-in-flight");
        let consumer = group.get_consumer(&topic, disk_conf()).await.unwrap();
        let in_flight = group.inner.load_full();
        drop(consumer);
        group.remove(&topic).await.unwrap();
        for n in 1..=3 {
            let res = timeout(Duration::from_secs(5), in_flight.broadcast(batch(n)))
                .await
                .expect("a send to the removed topic hung");
            assert!(matches!(res, Err(Error::Closed(_))), "{res:?}");
        }
    }

    /// A spill that fails waits for the consumer to have room instead of
    /// losing a payload already counted sent (#6).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_spill_waits_for_the_consumer() {
        ensure_db();
        let group = make_group_of::<Flaky>();
        let topic = unique_topic("spill-fails");
        let mut conf = disk_conf();
        if let config::Channel::Disk(disk) = &mut conf {
            disk.channel_buffer_size = 1;
        }
        let consumer = group.get_consumer(&topic, conf).await.unwrap();
        // One fills the consumer's input, the other cannot be spilled,
        // whichever order the bridge takes them in.
        for n in 1..=2 {
            let payload = Flaky {
                encodes: false,
                ..flaky(n)
            };
            group.broadcast(payload).await.unwrap();
        }
        assert_eq!(recv_lens(&consumer, 2).await, vec![1, 2]);
        assert_eq!(lost(&group, &topic), 0);
        group.remove(&topic).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nack_falls_back_to_the_stash_when_the_disk_write_fails() {
        ensure_db();
        let group = make_group_of::<Flaky>();
        let topic = unique_topic("nack-unwritable");
        let consumer = group.get_consumer(&topic, disk_conf()).await.unwrap();
        let payload = Flaky {
            encodes: false,
            ..flaky(3)
        };
        group.broadcast(payload).await.unwrap();
        let delivery = recv_one(&consumer).await.unwrap();
        consumer.nack(delivery).await;
        assert_eq!(consumer.stashed(), 1);
        assert_eq!(recv_lens(&consumer, 1).await, vec![3]);
        group.remove(&topic).await.unwrap();
    }

    /// `conf` capped at one byte, blocking.
    fn capped(conf: config::Channel) -> config::Channel {
        match conf {
            config::Channel::Disk(conf) => config::Channel::Disk(config::DiskChannel {
                max_disk_size: 1,
                non_blocking: false,
                ..conf
            }),
            config::Channel::Memory(_) => unreachable!(),
        }
    }

    /// An async send waiting on a full topic stops waiting once the topic
    /// is closed, and takes the payload past the cap (#40).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn async_send_to_a_full_blocking_disk_topic_returns_after_close_all() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("full-close");
        let _consumer = group
            .get_consumer(&topic, capped(write_through_conf(4)))
            .await
            .unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        let send = tokio::spawn({
            let group = group.clone();
            async move { group.broadcast(batch(2)).await }
        });
        tokio::time::sleep(STOP_POLL * 3).await;
        assert!(!send.is_finished(), "a full topic did not hold the send");
        group.close_all().await;
        let res = timeout(Duration::from_secs(5), send)
            .await
            .expect("the send still waits on a closed topic")
            .unwrap();
        assert!(res.is_ok(), "{res:?}");
        wait_for_keys(&topic, 2).await;
        remove_keyspaces(move |name| name == topic).await.unwrap();
    }

    /// A sync send waiting on a full topic that is then removed returns
    /// instead of waiting for `stop`, which only shutdown cancels: a
    /// processor thread used to stay there once its sink was deleted (#40).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_send_waiting_on_a_full_removed_disk_topic_returns() {
        ensure_db();
        let group = make_group();
        let topic = unique_topic("full-remove");
        let consumer = group
            .get_consumer(&topic, capped(write_through_conf(4)))
            .await
            .unwrap();
        group.send_to(&topic, batch(1)).unwrap();
        let sender = SenderChannel {
            inner: group.clone(),
        };
        let stop = CancellationToken::new();
        let send = {
            let (topic, stop) = (topic.clone(), stop.clone());
            std::thread::spawn(move || sender.send_to_until(&topic, batch(2), &stop))
        };
        tokio::time::sleep(STOP_POLL * 3).await;
        assert!(!send.is_finished(), "a full topic did not hold the send");
        drop(consumer);
        group.remove(&topic).await.unwrap();
        // Polled, not joined: a send that never returns must fail the test,
        // not hold the runtime's shutdown.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !send.is_finished() {
            assert!(
                Instant::now() < deadline,
                "the send still waits on a removed topic"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let res = send.join().unwrap();
        assert!(matches!(res, Err(Error::Closed(_))), "{res:?}");
    }
}
