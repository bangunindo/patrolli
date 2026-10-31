// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::payload::{AttrsBuilder, ResourceAttrKey, ScopeAttrKey};
use arrow::array::RecordBatch;
use arrow::datatypes::UInt16Type;

/// What [`BaseBuilder::qualified_scope_name`] reports for rows pushed before
/// any scope was set: the default scope, whose name is empty.
pub const UNSCOPED: &str = "patrolli.";

pub struct BaseBuilder {
    row_id: u16,
    cur_res_id: u16,
    cur_scope_id: u16,
    /// `patrolli.<scope name>` for each scope id handed out since the last finish.
    scope_names: Vec<String>,

    resource_map: ahash::HashMap<super::Resource, u16>,
    scope_map: ahash::HashMap<super::Scope, u16>,
    last_resource_id: u16,
    last_scope_id: u16,

    row_attr: AttrsBuilder<UInt16Type>,
    resource_attr: AttrsBuilder<UInt16Type>,
    scope_attr: AttrsBuilder<UInt16Type>,
}

pub struct FinishedBase {
    pub row_attr: RecordBatch,
    pub resource_attr: RecordBatch,
    pub scope_attr: RecordBatch,
}

impl Default for BaseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl BaseBuilder {
    pub fn new() -> Self {
        Self {
            row_id: 0,
            cur_res_id: 0,
            cur_scope_id: 0,
            scope_names: Vec::new(),

            resource_map: Default::default(),
            scope_map: Default::default(),
            last_resource_id: 0,
            last_scope_id: 0,

            row_attr: AttrsBuilder::new(),
            resource_attr: AttrsBuilder::new(),
            scope_attr: AttrsBuilder::new(),
        }
    }

    /// The current scope's name as rows carry it, `patrolli.<scope name>`.
    pub fn qualified_scope_name(&self) -> &str {
        self.scope_names
            .get(self.cur_scope_id as usize)
            .map_or(UNSCOPED, String::as_str)
    }

    /// [`Self::qualified_scope_name`] of every scope id, indexed by id.
    pub fn qualified_scope_names(&self) -> &[String] {
        &self.scope_names
    }

    pub fn scope_id(&self) -> u16 {
        self.cur_scope_id
    }

    pub fn resource_id(&self) -> u16 {
        self.cur_res_id
    }

    pub fn scope(&mut self, scope: super::Scope) {
        if let Some(id) = self.scope_map.get(&scope) {
            self.cur_scope_id = *id;
            return;
        }
        let id = self.last_scope_id;
        let attrs = &mut self.scope_attr;
        let mut id_buf = [0u8; uuid::fmt::Hyphenated::LENGTH];
        if let Some(listener_id) = &scope.listener_id {
            attrs.append_str(
                id,
                ScopeAttrKey::ListenerId.as_str(),
                listener_id.as_hyphenated().encode_lower(&mut id_buf),
            );
        }
        if let Some(listener_type) = &scope.listener_type {
            attrs.append_str(id, ScopeAttrKey::ListenerType.as_str(), listener_type);
        }
        if let Some(listener_name) = &scope.listener_name {
            attrs.append_str(id, ScopeAttrKey::ListenerName.as_str(), listener_name);
        }
        if let Some(monitor_type) = &scope.monitor_type {
            attrs.append_str(id, ScopeAttrKey::MonitorType.as_str(), monitor_type);
        }
        if let Some(node_id) = scope.node_id {
            attrs.append_str(
                id,
                ScopeAttrKey::NodeId.as_str(),
                node_id.as_hyphenated().encode_lower(&mut id_buf),
            );
        }
        self.scope_names.push(format!("patrolli.{}", scope.name));
        self.scope_map.insert(scope, self.last_scope_id);
        self.cur_scope_id = self.last_scope_id;
        self.last_scope_id += 1;
    }

    pub fn resource(&mut self, resource: super::Resource) {
        if let Some(id) = self.resource_map.get(&resource) {
            self.cur_res_id = *id;
            return;
        }
        let id = self.last_resource_id;
        let attrs = &mut self.resource_attr;
        let mut id_buf = [0u8; uuid::fmt::Hyphenated::LENGTH];
        attrs.append_str(
            id,
            ResourceAttrKey::ServiceName.as_str(),
            &resource.host_name,
        );
        attrs.append_str(
            id,
            ResourceAttrKey::ServiceInstanceId.as_str(),
            resource.host_id.as_hyphenated().encode_lower(&mut id_buf),
        );
        attrs.append_str(id, ResourceAttrKey::HostName.as_str(), &resource.host_name);
        attrs.append_str(
            id,
            ResourceAttrKey::HostId.as_str(),
            resource.host_id.as_hyphenated().encode_lower(&mut id_buf),
        );
        attrs.append_str(id, ResourceAttrKey::ItemName.as_str(), &resource.item_name);
        attrs.append_str(
            id,
            ResourceAttrKey::ItemId.as_str(),
            resource.item_id.as_hyphenated().encode_lower(&mut id_buf),
        );
        if let Some(endpoint_id) = &resource.endpoint_id {
            attrs.append_str(
                id,
                ResourceAttrKey::EndpointId.as_str(),
                endpoint_id.as_hyphenated().encode_lower(&mut id_buf),
            );
        }
        if let Some(endpoint_name) = &resource.endpoint_name {
            attrs.append_str(id, ResourceAttrKey::EndpointName.as_str(), endpoint_name);
        }
        if let Some(endpoint_type) = &resource.endpoint_type {
            attrs.append_str(id, ResourceAttrKey::EndpointType.as_str(), endpoint_type);
        }
        if let Some(host_timezone) = &resource.host_timezone {
            attrs.append_str(id, ResourceAttrKey::HostTimezone.as_str(), host_timezone);
        }
        self.resource_map.insert(resource, self.last_resource_id);
        self.cur_res_id = self.last_resource_id;
        self.last_resource_id += 1;
    }

    pub fn finish(&mut self) -> FinishedBase {
        // panic means logic errors
        let row_attr = self.row_attr.finish().unwrap();
        let resource_attr = self.resource_attr.finish().unwrap();
        let scope_attr = self.scope_attr.finish().unwrap();
        self.row_id = 0;
        self.cur_res_id = 0;
        self.cur_scope_id = 0;
        self.last_resource_id = 0;
        self.last_scope_id = 0;
        // in theory, map's capacity wouldn't exceed u16::MAX, so it's safe to not shrink
        // the map's capacity
        self.resource_map.clear();
        self.scope_map.clear();
        self.scope_names.clear();
        FinishedBase {
            row_attr,
            resource_attr,
            scope_attr,
        }
    }

    /// Root rows appended so far. Bounded by `u16::MAX` — [`Self::next_row_id`]
    /// panics past that, so callers must flush before reaching it.
    pub fn num_rows(&self) -> usize {
        self.row_id as usize
    }

    pub fn is_empty(&self) -> bool {
        self.row_id == 0
    }

    pub fn next_row_id(&mut self) -> u16 {
        if self.row_id == u16::MAX {
            panic!("log id overflow");
        }
        let row_id = self.row_id;
        self.row_id += 1;
        row_id
    }

    pub fn append_row_attr(&mut self, id: u16, key: &str, val: &super::Value) {
        val.append_to(&mut self.row_attr, id, key);
    }
}
