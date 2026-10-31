// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::config::endpoint;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tracing::debug;
use uuid::Uuid;

pub(super) static ENDPOINT_CACHE: LazyLock<Arc<RwLock<Endpoint>>> =
    LazyLock::new(|| Arc::new(RwLock::new(Endpoint::new())));

pub struct Endpoint {
    data: FxHashMap<Uuid, Arc<endpoint::Endpoint>>,
    last_compact: Instant,
}

impl Default for Endpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl Endpoint {
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

    pub fn replace(&mut self, endpoints: Vec<endpoint::Endpoint>) {
        self.data.clear();
        for endpoint in endpoints {
            self.insert(endpoint);
        }
        debug!(count = self.data.len(), "endpoint cache replaced");
    }

    pub fn clear(&mut self) {
        self.data = FxHashMap::default();
    }

    pub fn insert(&mut self, endpoint: endpoint::Endpoint) {
        self.data.insert(endpoint.id, Arc::new(endpoint));
    }

    pub fn remove(&mut self, id: &Uuid) -> Option<Arc<endpoint::Endpoint>> {
        let ep = self.data.remove(id);
        if self.last_compact.elapsed() > Duration::from_mins(15)
            && self.data.len() * 2 < self.data.capacity()
        {
            self.data.shrink_to_fit();
        }
        ep
    }

    pub fn get(&self, id: &Uuid) -> Option<&Arc<endpoint::Endpoint>> {
        self.data.get(id)
    }
}
