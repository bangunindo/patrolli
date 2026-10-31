// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

mod channel;
pub mod codec;
mod consumer;

use crate::common::internal_metrics::{Channel as Metrics, ChannelState};
use rustc_hash::{FxBuildHasher, FxHashMap as HashMap};
use std::fmt::Display;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

pub use channel::{
    AdminChannel, Error, Message, SenderChannel, Topic, init_database, new, remove_keyspaces,
};
pub use codec::Codec;
pub use consumer::{AckToken, Consumer, Delivery};

type PHashMap<K, V> = papaya::HashMap<K, V, FxBuildHasher>;

struct InternalMetrics<K> {
    total_messages: AtomicUsize,
    total_batches: AtomicUsize,
    total_time_spent: AtomicU64,
    messages_sent: PHashMap<K, AtomicUsize>,
    messages_dropped: PHashMap<K, AtomicUsize>,
    batches_sent: PHashMap<K, AtomicUsize>,
    batches_dropped: PHashMap<K, AtomicUsize>,
    /// Accepted (counted sent), then lost inside the topic: a disk value
    /// that does not decode, a spill that fails with no consumer left.
    messages_lost: PHashMap<K, AtomicUsize>,
    batches_lost: PHashMap<K, AtomicUsize>,
    send_wait_time: PHashMap<K, AtomicU64>,
}

impl<K> Default for InternalMetrics<K> {
    fn default() -> Self {
        Self {
            total_messages: AtomicUsize::new(0),
            total_batches: AtomicUsize::new(0),
            total_time_spent: AtomicU64::new(0),
            messages_sent: PHashMap::default(),
            messages_dropped: PHashMap::default(),
            batches_sent: PHashMap::default(),
            batches_dropped: PHashMap::default(),
            messages_lost: PHashMap::default(),
            batches_lost: PHashMap::default(),
            send_wait_time: PHashMap::default(),
        }
    }
}

impl<K: Hash + Eq + Clone + Display> InternalMetrics<K> {
    fn get(&self, channels: HashMap<String, ChannelState>) -> Metrics {
        let total_messages = self.total_messages.load(Ordering::Relaxed);
        let total_batches = self.total_batches.load(Ordering::Relaxed);
        let total_time_spent = Duration::from_nanos(self.total_time_spent.load(Ordering::Relaxed));
        let messages_sent = Self::clone_map(&self.messages_sent);
        let messages_dropped = Self::clone_map(&self.messages_dropped);
        let batches_sent = Self::clone_map(&self.batches_sent);
        let batches_dropped = Self::clone_map(&self.batches_dropped);
        let messages_lost = Self::clone_map(&self.messages_lost);
        let batches_lost = Self::clone_map(&self.batches_lost);
        let send_wait_time = Self::clone_map_duration(&self.send_wait_time);

        Metrics {
            channels,
            total_messages,
            total_batches,
            messages_sent,
            messages_dropped,
            batches_sent,
            batches_dropped,
            messages_lost,
            batches_lost,
            total_time_spent,
            send_wait_time,
        }
    }

    fn add_total_time(&self, duration: Duration) {
        self.total_time_spent
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    fn add_total_messages(&self, count: usize) {
        self.total_messages.fetch_add(count, Ordering::Relaxed);
    }

    fn add_total_batches(&self, count: usize) {
        self.total_batches.fetch_add(count, Ordering::Relaxed);
    }

    fn clone_map(map: &PHashMap<K, AtomicUsize>) -> HashMap<String, usize> {
        let mut new_map = HashMap::default();
        for (k, v) in map.pin().iter() {
            new_map.insert(k.to_string(), v.load(Ordering::Relaxed));
        }
        new_map
    }

    fn clone_map_duration(map: &PHashMap<K, AtomicU64>) -> HashMap<String, Duration> {
        let mut new_map = HashMap::default();
        for (k, v) in map.pin().iter() {
            new_map.insert(
                k.to_string(),
                Duration::from_nanos(v.load(Ordering::Relaxed)),
            );
        }
        new_map
    }

    fn add_to_map(map: &PHashMap<K, AtomicUsize>, topic: &K, count: usize, replace: bool) {
        let in_map = map.pin();
        match (replace, in_map.get(topic)) {
            (true, Some(value)) => {
                value.store(count, Ordering::Relaxed);
            }
            (false, Some(value)) => {
                value.fetch_add(count, Ordering::Relaxed);
            }
            (_, None) => {
                in_map.insert(topic.clone(), AtomicUsize::new(count));
            }
        };
    }

    fn add_to_duration_map(map: &PHashMap<K, AtomicU64>, topic: &K, duration: u64, replace: bool) {
        let in_map = map.pin();
        match (replace, in_map.get(topic)) {
            (true, Some(value)) => {
                value.store(duration, Ordering::Relaxed);
            }
            (false, Some(value)) => {
                value.fetch_add(duration, Ordering::Relaxed);
            }
            (_, None) => {
                in_map.insert(topic.clone(), AtomicU64::new(duration));
            }
        };
    }

    fn add_send_wait_time(&self, topic: &K, duration: Duration) {
        Self::add_to_duration_map(
            &self.send_wait_time,
            topic,
            duration.as_nanos() as u64,
            false,
        );
    }

    fn add_messages_sent(&self, topic: &K, count: usize) {
        Self::add_to_map(&self.messages_sent, topic, count, false);
    }

    fn add_messages_dropped(&self, topic: &K, count: usize) {
        Self::add_to_map(&self.messages_dropped, topic, count, false);
    }

    fn add_batches_sent(&self, topic: &K, count: usize) {
        Self::add_to_map(&self.batches_sent, topic, count, false);
    }

    fn add_batches_dropped(&self, topic: &K, count: usize) {
        Self::add_to_map(&self.batches_dropped, topic, count, false);
    }

    fn add_messages_lost(&self, topic: &K, count: usize) {
        Self::add_to_map(&self.messages_lost, topic, count, false);
    }

    fn add_batches_lost(&self, topic: &K, count: usize) {
        Self::add_to_map(&self.batches_lost, topic, count, false);
    }

    fn remove_channel(&self, topic: &K) {
        self.messages_sent.pin().remove(topic);
        self.messages_dropped.pin().remove(topic);
        self.batches_sent.pin().remove(topic);
        self.batches_dropped.pin().remove(topic);
        self.messages_lost.pin().remove(topic);
        self.batches_lost.pin().remove(topic);
        self.send_wait_time.pin().remove(topic);
    }

    fn remove_all_channels(&self) {
        self.messages_sent.pin().clear();
        self.messages_dropped.pin().clear();
        self.batches_sent.pin().clear();
        self.batches_dropped.pin().clear();
        self.messages_lost.pin().clear();
        self.batches_lost.pin().clear();
        self.send_wait_time.pin().clear();
    }
}
