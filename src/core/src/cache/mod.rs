// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use parking_lot::RwLock;
use std::sync::Arc;

pub mod endpoint;
pub mod host;
pub mod item;
pub mod listener;
pub mod lookup_table;
pub mod processor;

#[derive(Clone)]
pub struct Cache {
    pub item: Arc<RwLock<item::Item>>,
    pub endpoint: Arc<RwLock<endpoint::Endpoint>>,
    pub processor: Arc<processor::Processor>,
    pub listener: Arc<RwLock<listener::Listener>>,
    pub host: Arc<RwLock<host::Host>>,
    /// no outer lock: the registry manages its own `ArcSwap` internally
    pub lookup_table: Arc<lookup_table::LookupTable>,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    pub fn new() -> Self {
        Cache {
            item: Arc::new(RwLock::new(item::Item::new())),
            endpoint: Arc::new(RwLock::new(endpoint::Endpoint::new())),
            processor: Arc::new(processor::Processor::new()),
            listener: Arc::new(RwLock::new(listener::Listener::new())),
            host: Arc::new(RwLock::new(host::Host::new())),
            lookup_table: Arc::new(lookup_table::LookupTable::new()),
        }
    }
}

pub fn get_cache() -> Cache {
    Cache {
        item: item::ITEM_CACHE.clone(),
        endpoint: endpoint::ENDPOINT_CACHE.clone(),
        processor: processor::PROCESSOR_CACHE.clone(),
        listener: listener::LISTENER_CACHE.clone(),
        host: host::HOST_CACHE.clone(),
        lookup_table: lookup_table::LOOKUP_TABLE_CACHE.clone(),
    }
}
