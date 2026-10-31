// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::config::item::ListenerItemMatchStrategyMethod;
use crate::config::{item, listener};
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, LazyLock};
use tracing::warn;
use uuid::Uuid;

pub(super) static LISTENER_CACHE: LazyLock<Arc<RwLock<Listener>>> =
    LazyLock::new(|| Arc::new(RwLock::new(Listener::new())));

#[derive(Debug)]
pub struct ItemData {
    pub item: Arc<item::Item>,
    pub monitored_by: Option<Uuid>,
}

#[derive(Debug, Default)]
struct ListenerData {
    listener: Option<Arc<listener::Listener>>,
    items_index: FxHashMap<Uuid, ListenerItemMatchStrategyMethod>,
    items: BTreeMap<(ListenerItemMatchStrategyMethod, Uuid), ItemData>,
}

impl Clone for ItemData {
    fn clone(&self) -> Self {
        Self {
            item: self.item.clone(),
            monitored_by: self.monitored_by,
        }
    }
}

// items and endpoints here are only for listener's
pub struct Listener {
    listeners: FxHashMap<Uuid, ListenerData>,
    // used in item with match strategy EndpointSourceIp
    endpoint_ip: FxHashMap<Uuid, IpAddr>,
    item_to_listener: FxHashMap<Uuid, Uuid>,
}

impl Default for Listener {
    fn default() -> Self {
        Self::new()
    }
}

impl Listener {
    pub fn new() -> Self {
        Self {
            listeners: FxHashMap::default(),
            endpoint_ip: FxHashMap::default(),
            item_to_listener: FxHashMap::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.listeners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.listeners.is_empty()
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn insert(&mut self, listener: listener::Listener) {
        let listener_entry = self.listeners.entry(listener.id).or_default();
        listener_entry.listener = Some(Arc::new(listener));
    }

    pub fn compact(&mut self) {
        self.listeners.shrink_to_fit();
        for listener in self.listeners.values_mut() {
            listener.items_index.shrink_to_fit();
        }
        self.endpoint_ip.shrink_to_fit();
    }

    pub fn insert_item(&mut self, item: item::Item, monitored_by: Option<Uuid>) {
        self.remove_item(&item.id);
        let Some(listener_id) = item.listener_id else {
            return;
        };
        let match_strategy: ListenerItemMatchStrategyMethod =
            if let item::Config::Listener(config) = &item.config {
                (&config.match_strategy).into()
            } else {
                warn!(
                    item_id = %item.id,
                    item = %item.name,
                    listener_id = %listener_id,
                    "item has listener_id but no listener config, ignored"
                );
                return;
            };
        let item_id = item.id;
        let entry = self.listeners.entry(listener_id).or_default();
        entry.items_index.insert(item_id, match_strategy);
        entry.items.insert(
            (match_strategy, item_id),
            ItemData {
                item: Arc::new(item),
                monitored_by,
            },
        );
        self.item_to_listener.insert(item_id, listener_id);
    }

    pub fn insert_ip(&mut self, endpoint_id: Uuid, ip: IpAddr) {
        self.endpoint_ip.insert(endpoint_id, ip);
    }

    pub fn remove(&mut self, listener_id: &Uuid) -> Option<Arc<listener::Listener>> {
        self.listeners
            .remove(listener_id)
            .and_then(|listeners| listeners.listener)
    }

    pub fn remove_item(&mut self, item_id: &Uuid) -> Option<Arc<item::Item>> {
        let listener_id = self.item_to_listener.get(item_id).copied()?;
        let listener = self.listeners.get_mut(&listener_id)?;
        let match_strategy = listener.items_index.get(item_id).copied()?;
        if let Some(item) = listener.items.remove(&(match_strategy, *item_id)) {
            listener.items_index.remove(item_id);
            self.item_to_listener.remove(item_id);
            Some(item.item)
        } else {
            None
        }
    }

    pub fn remove_ip(&mut self, endpoint_id: &Uuid) {
        self.endpoint_ip.remove(endpoint_id);
    }

    pub fn get(&self, listener_id: &Uuid) -> Option<&Arc<listener::Listener>> {
        self.listeners
            .get(listener_id)
            .and_then(|listeners| listeners.listener.as_ref())
    }

    pub fn get_items(
        &self,
        listener_id: &Uuid,
    ) -> Option<impl ExactSizeIterator<Item = &ItemData>> {
        Some(self.listeners.get(listener_id)?.items.values())
    }

    pub fn get_endpoint_ip(&self, endpoint_id: &Uuid) -> Option<&IpAddr> {
        self.endpoint_ip.get(endpoint_id)
    }
}
