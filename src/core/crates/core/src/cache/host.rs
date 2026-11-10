// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::config::host;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tracing::debug;
use uuid::Uuid;

pub(super) static HOST_CACHE: LazyLock<Arc<RwLock<Host>>> =
    LazyLock::new(|| Arc::new(RwLock::new(Host::new())));

pub struct Host {
    data: FxHashMap<Uuid, Arc<host::Host>>,
    last_compact: Instant,
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Host {
    pub fn new() -> Self {
        Self {
            data: FxHashMap::default(),
            last_compact: Instant::now(),
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn replace(&mut self, hosts: Vec<host::Host>) {
        self.data.clear();
        for host in hosts {
            self.insert(host);
        }
        debug!(count = self.data.len(), "host cache replaced");
    }

    pub fn insert(&mut self, host: host::Host) {
        self.data.insert(host.id, Arc::new(host));
    }

    pub fn clear(&mut self) {
        self.data = FxHashMap::default();
    }

    pub fn remove(&mut self, id: &Uuid) -> Option<Arc<host::Host>> {
        let ep = self.data.remove(id);
        if self.last_compact.elapsed() > Duration::from_mins(15)
            && self.data.len() * 2 < self.data.capacity()
        {
            self.data.shrink_to_fit();
        }
        ep
    }

    pub fn get(&self, id: &Uuid) -> Option<&Arc<host::Host>> {
        self.data.get(id)
    }
}
