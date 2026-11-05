// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// What each config table's trigger writes into `cluster_logs`, the only way a
// change reaches the nodes. A node applies the latest row per resource: an
// upsert re-reads the resource through the watcher's own filters, a delete
// removes it from the caches. An upsert that reads nothing removes nothing, so
// a missing row and an upsert where a delete is due both leave a node running
// stale config (N3: disabling a sink logged nothing).
//
// Every case runs against one committed `World`, inside a transaction it rolls
// back: its setup, an emptied log, its action. Then the rows the action logged
// must equal the expected ones (as a multiset), and each node's view through
// `query::cluster_logs` (what the watcher reads) the last expected row of each
// resource. All mismatches are reported together.
//
// Per-node resources (`host`, `host_endpoint`, `item`) go to the host's
// `monitored_by` only. The rest go to every node: listener items and endpoints
// because any node can receive listener traffic, a cluster node to every node
// but itself.

mod setup;

use anyhow::{Context, Result};
use deadpool_postgres::Transaction;
use patrolli::db::query;
use patrolli::db::schema::{ClusterLogResource, ClusterLogs};
use sea_query::PostgresQueryBuilder;
use sea_query_postgres::PostgresBinder;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::str::FromStr;
use uuid::Uuid;

const UPSERT: i16 = 1;
const DELETE: i16 = 2;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    node: Uuid,
    resource: String,
    id: Uuid,
    op: i16,
}

fn on(node: Uuid, resource: &str, id: Uuid, op: i16) -> Vec<Row> {
    vec![Row {
        node,
        resource: resource.to_string(),
        id,
        op,
    }]
}

/// The committed configuration every case starts from.
///
/// - `host` (enabled, on node 1) has `endpoint` (enabled) and `endpoint_off`
///   (disabled). On `endpoint`: `item` (snmp), `item_off` (disabled),
///   `listener_item`, and `child_item`, inherited from `template_item`; on
///   `endpoint_off`: `item_on_endpoint_off`. `attribute_item` is a listener
///   item with no endpoint.
/// - `host_off` (disabled, on node 1) has `endpoint_host_off` with
///   `item_host_off` and `listener_item_host_off`.
/// - `host_free` (enabled, on no node) has `endpoint_free` with
///   `listener_item_free`.
/// - `processor_2` takes `processor` as its input, so do `sink` and `sink_off`.
///
/// `x` and `y` are ids for rows a case creates; every case rolls back, so they
/// never collide.
struct World {
    names: HashMap<Uuid, &'static str>,
    n1: Uuid,
    n2: Uuid,
    listener: Uuid,
    listener_off: Uuid,
    processor: Uuid,
    processor_2: Uuid,
    sink: Uuid,
    sink_off: Uuid,
    lookup_table: Uuid,
    lookup_table_off: Uuid,
    host: Uuid,
    host_ht: Uuid,
    endpoint: Uuid,
    endpoint_off: Uuid,
    item: Uuid,
    item_off: Uuid,
    item_on_endpoint_off: Uuid,
    listener_item: Uuid,
    attribute_item: Uuid,
    template: Uuid,
    template_ht: Uuid,
    template_item: Uuid,
    host_template_ht: Uuid,
    child_item: Uuid,
    host_off: Uuid,
    host_off_ht: Uuid,
    endpoint_host_off: Uuid,
    item_host_off: Uuid,
    listener_item_host_off: Uuid,
    host_free: Uuid,
    host_free_ht: Uuid,
    endpoint_free: Uuid,
    listener_item_free: Uuid,
    x: Uuid,
    y: Uuid,
}

impl World {
    fn new() -> Self {
        let mut names = HashMap::new();
        let mut id = |name| {
            let id = Uuid::now_v7();
            names.insert(id, name);
            id
        };
        let n1 = id("node_1");
        let n2 = id("node_2");
        let listener = id("listener");
        let listener_off = id("listener_off");
        let processor = id("processor");
        let processor_2 = id("processor_2");
        let sink = id("sink");
        let sink_off = id("sink_off");
        let lookup_table = id("lookup_table");
        let lookup_table_off = id("lookup_table_off");
        let host = id("host");
        let host_ht = id("host_ht");
        let endpoint = id("endpoint");
        let endpoint_off = id("endpoint_off");
        let item = id("item");
        let item_off = id("item_off");
        let item_on_endpoint_off = id("item_on_endpoint_off");
        let listener_item = id("listener_item");
        let attribute_item = id("attribute_item");
        let template = id("template");
        let template_ht = id("template_ht");
        let template_item = id("template_item");
        let host_template_ht = id("host_template_ht");
        let child_item = id("child_item");
        let host_off = id("host_off");
        let host_off_ht = id("host_off_ht");
        let endpoint_host_off = id("endpoint_host_off");
        let item_host_off = id("item_host_off");
        let listener_item_host_off = id("listener_item_host_off");
        let host_free = id("host_free");
        let host_free_ht = id("host_free_ht");
        let endpoint_free = id("endpoint_free");
        let listener_item_free = id("listener_item_free");
        let x = id("x");
        let y = id("y");
        Self {
            names,
            n1,
            n2,
            listener,
            listener_off,
            processor,
            processor_2,
            sink,
            sink_off,
            lookup_table,
            lookup_table_off,
            host,
            host_ht,
            endpoint,
            endpoint_off,
            item,
            item_off,
            item_on_endpoint_off,
            listener_item,
            attribute_item,
            template,
            template_ht,
            template_item,
            host_template_ht,
            child_item,
            host_off,
            host_off_ht,
            endpoint_host_off,
            item_host_off,
            listener_item_host_off,
            host_free,
            host_free_ht,
            endpoint_free,
            listener_item_free,
            x,
            y,
        }
    }

    fn sql(&self) -> Vec<String> {
        let w = self;
        vec![
            insert_node(w.n1),
            insert_node(w.n2),
            insert_listener(w.listener, 1),
            insert_listener(w.listener_off, 0),
            insert_processor(w.processor),
            insert_processor(w.processor_2),
            processor_input(w.processor_2, w.processor),
            insert_sink(w.sink, 1),
            insert_sink(w.sink_off, 0),
            sink_input(w.sink, w.processor),
            sink_input(w.sink_off, w.processor),
            insert_lookup_table(w.lookup_table, 1),
            insert_lookup_table(w.lookup_table_off, 0),
            insert_host(w.host, Some(w.n1), 1),
            insert_host(w.host_off, Some(w.n1), 0),
            insert_host(w.host_free, None, 1),
            format!(
                "INSERT INTO templates (template_id, name) VALUES ('{}', 'template')",
                w.template
            ),
            host_template(w.host_ht, Some(w.host), None),
            host_template(w.template_ht, None, Some(w.template)),
            host_template(w.host_template_ht, Some(w.host), Some(w.template)),
            host_template(w.host_off_ht, Some(w.host_off), None),
            host_template(w.host_free_ht, Some(w.host_free), None),
            insert_endpoint(w.endpoint, w.host, 1),
            insert_endpoint(w.endpoint_off, w.host, 0),
            insert_endpoint(w.endpoint_host_off, w.host_off, 1),
            insert_endpoint(w.endpoint_free, w.host_free, 1),
            insert_item(w.item, w.host_ht, Some(w.endpoint), None, 1),
            insert_item(w.item_off, w.host_ht, Some(w.endpoint), None, 0),
            insert_item(
                w.item_on_endpoint_off,
                w.host_ht,
                Some(w.endpoint_off),
                None,
                1,
            ),
            insert_item(
                w.listener_item,
                w.host_ht,
                Some(w.endpoint),
                Some(w.listener),
                1,
            ),
            insert_item(w.attribute_item, w.host_ht, None, Some(w.listener), 1),
            insert_item(w.template_item, w.template_ht, None, None, 1),
            format!(
                "INSERT INTO items (item_id, host_template_id, template_item_id, endpoint_id, type, \
                 adm_status, last_updated) VALUES ('{}', '{}', '{}', '{}', 'snmp', 1, now())",
                w.child_item, w.host_template_ht, w.template_item, w.endpoint
            ),
            insert_item(
                w.item_host_off,
                w.host_off_ht,
                Some(w.endpoint_host_off),
                None,
                1,
            ),
            insert_item(
                w.listener_item_host_off,
                w.host_off_ht,
                Some(w.endpoint_host_off),
                Some(w.listener),
                1,
            ),
            insert_item(
                w.listener_item_free,
                w.host_free_ht,
                Some(w.endpoint_free),
                Some(w.listener),
                1,
            ),
            "DELETE FROM cluster_logs".to_string(),
        ]
    }

    /// One row per node.
    fn all(&self, resource: &str, id: Uuid, op: i16) -> Vec<Row> {
        [on(self.n1, resource, id, op), on(self.n2, resource, id, op)].concat()
    }

    /// `host` and everything active under it, to its node.
    fn host_tree(&self, node: Uuid, op: i16) -> Vec<Row> {
        [
            on(node, "host", self.host, op),
            on(node, "host_endpoint", self.endpoint, op),
            on(node, "item", self.item, op),
            on(node, "item", self.listener_item, op),
            on(node, "item", self.child_item, op),
            on(node, "item", self.attribute_item, op),
        ]
        .concat()
    }

    /// `host`'s listener items and their endpoint, to every node.
    fn host_listeners(&self, op: i16) -> Vec<Row> {
        [
            self.all("listener_endpoint", self.endpoint, op),
            self.all("listener_item", self.listener_item, op),
            self.all("listener_item", self.attribute_item, op),
        ]
        .concat()
    }

    fn name(&self, id: Uuid) -> String {
        self.names
            .get(&id)
            .map_or_else(|| id.to_string(), |n| n.to_string())
    }

    fn show(&self, row: &Row) -> String {
        let op = match row.op {
            UPSERT => "upsert".to_string(),
            DELETE => "delete".to_string(),
            op => format!("op {op}"),
        };
        format!(
            "{} {} {} {op}",
            self.name(row.node),
            row.resource,
            self.name(row.id)
        )
    }
}

fn opt(id: Option<Uuid>) -> String {
    id.map_or_else(|| "NULL".to_string(), |id| format!("'{id}'"))
}

fn insert_node(id: Uuid) -> String {
    format!(
        "INSERT INTO cluster_nodes (node_id, name, op_status, last_heartbeat, metadata) \
         VALUES ('{id}', 'node', 1, now(), '{{}}')"
    )
}

fn insert_listener(id: Uuid, adm_status: i16) -> String {
    format!(
        "INSERT INTO listeners (listener_id, name, type, adm_status, last_updated, config) \
         VALUES ('{id}', 'listener', 'syslog', {adm_status}, now(), '{{}}')"
    )
}

fn insert_processor(id: Uuid) -> String {
    format!(
        "INSERT INTO processors (processor_id, name, type, adm_status, last_updated, config) \
         VALUES ('{id}', 'processor', 'null', 1, now(), 'null')"
    )
}

fn processor_input(processor: Uuid, input: Uuid) -> String {
    format!(
        "INSERT INTO processor_inputs (processor_id, input_id) VALUES ('{processor}', '{input}')"
    )
}

fn insert_sink(id: Uuid, adm_status: i16) -> String {
    format!(
        "INSERT INTO sinks (sink_id, name, type, adm_status, last_updated, config) \
         VALUES ('{id}', 'sink', 'file', {adm_status}, now(), '{{}}')"
    )
}

fn sink_input(sink: Uuid, input: Uuid) -> String {
    format!("INSERT INTO sink_inputs (sink_id, input_id) VALUES ('{sink}', '{input}')")
}

fn insert_lookup_table(id: Uuid, adm_status: i16) -> String {
    format!(
        "INSERT INTO lookup_tables (lookup_table_id, name, type, adm_status, last_updated, \
         source, key_type, key) \
         VALUES ('{id}', 'lookup_table', 'file', {adm_status}, now(), '{{}}', 'hash', '{{}}')"
    )
}

fn insert_host(id: Uuid, monitored_by: Option<Uuid>, adm_status: i16) -> String {
    format!(
        "INSERT INTO hosts (host_id, name, adm_status, last_updated, monitored_by) \
         VALUES ('{id}', 'host', {adm_status}, now(), {})",
        opt(monitored_by)
    )
}

fn host_template(id: Uuid, host: Option<Uuid>, template: Option<Uuid>) -> String {
    format!(
        "INSERT INTO host_templates (host_template_id, host_id, template_id) VALUES ('{id}', {}, {})",
        opt(host),
        opt(template)
    )
}

fn insert_endpoint(id: Uuid, host: Uuid, adm_status: i16) -> String {
    format!(
        "INSERT INTO host_endpoints (endpoint_id, host_id, name, type, adm_status, last_updated, \
         config) VALUES ('{id}', '{host}', 'endpoint', 'snmp_v2c', {adm_status}, now(), \
         '{{\"address\": \"10.0.0.1:161\"}}')"
    )
}

/// An snmp item, or a listener item when `listener` is set.
fn insert_item(
    id: Uuid,
    host_template: Uuid,
    endpoint: Option<Uuid>,
    listener: Option<Uuid>,
    adm_status: i16,
) -> String {
    let kind = if listener.is_some() {
        "listener"
    } else {
        "snmp"
    };
    format!(
        "INSERT INTO items (item_id, host_template_id, endpoint_id, listener_id, type, adm_status, \
         name, schedule, timeout_ms, last_updated, config) VALUES ('{id}', '{host_template}', {}, \
         {}, '{kind}', {adm_status}, 'item', '* * * * * * *', 1000, now(), '{{}}')",
        opt(endpoint),
        opt(listener)
    )
}

fn key(table: &str) -> &'static str {
    match table {
        "listeners" => "listener_id",
        "sinks" => "sink_id",
        "processors" => "processor_id",
        "lookup_tables" => "lookup_table_id",
        "cluster_nodes" => "node_id",
        "hosts" => "host_id",
        "host_endpoints" => "endpoint_id",
        "items" => "item_id",
        _ => panic!("no key column for {table}"),
    }
}

fn set(table: &str, id: Uuid, assignments: &str) -> String {
    format!(
        "UPDATE {table} SET {assignments} WHERE {} = '{id}'",
        key(table)
    )
}

fn delete(table: &str, id: Uuid) -> String {
    format!("DELETE FROM {table} WHERE {} = '{id}'", key(table))
}

struct Case {
    name: &'static str,
    setup: Vec<String>,
    action: Vec<String>,
    /// In the order the action logs them, which decides each resource's
    /// latest row.
    expect: Vec<Row>,
}

fn case<const N: usize>(name: &'static str, action: [String; N], expect: Vec<Vec<Row>>) -> Case {
    Case {
        name,
        setup: Vec::new(),
        action: action.into(),
        expect: expect.concat(),
    }
}

impl Case {
    fn given<const N: usize>(mut self, setup: [String; N]) -> Self {
        self.setup = setup.into();
        self
    }
}

fn cases(w: &World) -> Vec<Case> {
    let (n1, n2, x, y) = (w.n1, w.n2, w.x, w.y);
    vec![
        // ── listeners, sinks and lookup tables: every node; a disable is a delete
        case(
            "listener: insert",
            [insert_listener(x, 1)],
            vec![w.all("listener", x, UPSERT)],
        ),
        case("listener: insert disabled", [insert_listener(x, 0)], vec![]),
        case(
            "listener: update",
            [set("listeners", w.listener, "name = 'renamed'")],
            vec![w.all("listener", w.listener, UPSERT)],
        ),
        case(
            "listener: disable",
            [set("listeners", w.listener, "adm_status = 0")],
            vec![w.all("listener", w.listener, DELETE)],
        ),
        case(
            "listener: enable",
            [set("listeners", w.listener_off, "adm_status = 1")],
            vec![w.all("listener", w.listener_off, UPSERT)],
        ),
        case(
            "listener: update while disabled",
            [set("listeners", w.listener_off, "name = 'renamed'")],
            vec![],
        ),
        case(
            "listener: delete",
            [delete("listeners", x)],
            vec![w.all("listener", x, DELETE)],
        )
        .given([insert_listener(x, 1)]),
        case(
            "listener: delete while disabled",
            [delete("listeners", w.listener_off)],
            vec![],
        ),
        case(
            "sink: insert",
            [insert_sink(x, 1)],
            vec![w.all("sink", x, UPSERT)],
        ),
        case("sink: insert disabled", [insert_sink(x, 0)], vec![]),
        case(
            "sink: update",
            [set("sinks", w.sink, "name = 'renamed'")],
            vec![w.all("sink", w.sink, UPSERT)],
        ),
        case(
            "sink: disable (N3)",
            [set("sinks", w.sink, "adm_status = 0")],
            vec![w.all("sink", w.sink, DELETE)],
        ),
        case(
            "sink: enable",
            [set("sinks", w.sink_off, "adm_status = 1")],
            vec![w.all("sink", w.sink_off, UPSERT)],
        ),
        case(
            "sink: update while disabled",
            [set("sinks", w.sink_off, "name = 'renamed'")],
            vec![],
        ),
        case(
            "sink: delete",
            [delete("sinks", x)],
            vec![w.all("sink", x, DELETE)],
        )
        .given([insert_sink(x, 1)]),
        case("sink: delete while disabled", [delete("sinks", x)], vec![])
            .given([insert_sink(x, 0)]),
        case(
            "sink: add an input",
            [sink_input(w.sink, w.processor_2)],
            vec![w.all("sink", w.sink, UPSERT)],
        ),
        case(
            "sink: remove an input",
            [format!(
                "DELETE FROM sink_inputs WHERE sink_id = '{}'",
                w.sink
            )],
            vec![w.all("sink", w.sink, UPSERT)],
        ),
        case(
            "sink: add an input while disabled",
            [sink_input(w.sink_off, w.processor_2)],
            vec![],
        ),
        // the API's delete: the inputs' upsert must not outlive the delete (N35)
        case(
            "sink: delete with its inputs",
            [
                format!("DELETE FROM sink_inputs WHERE sink_id = '{}'", w.sink),
                delete("sinks", w.sink),
            ],
            vec![w.all("sink", w.sink, UPSERT), w.all("sink", w.sink, DELETE)],
        ),
        case(
            "lookup table: insert",
            [insert_lookup_table(x, 1)],
            vec![w.all("lookup_table", x, UPSERT)],
        ),
        case(
            "lookup table: insert disabled",
            [insert_lookup_table(x, 0)],
            vec![],
        ),
        case(
            "lookup table: update",
            [set("lookup_tables", w.lookup_table, "name = 'renamed'")],
            vec![w.all("lookup_table", w.lookup_table, UPSERT)],
        ),
        case(
            "lookup table: disable",
            [set("lookup_tables", w.lookup_table, "adm_status = 0")],
            vec![w.all("lookup_table", w.lookup_table, DELETE)],
        ),
        case(
            "lookup table: enable",
            [set("lookup_tables", w.lookup_table_off, "adm_status = 1")],
            vec![w.all("lookup_table", w.lookup_table_off, UPSERT)],
        ),
        case(
            "lookup table: update while disabled",
            [set("lookup_tables", w.lookup_table_off, "name = 'renamed'")],
            vec![],
        ),
        case(
            "lookup table: delete",
            [delete("lookup_tables", w.lookup_table)],
            vec![w.all("lookup_table", w.lookup_table, DELETE)],
        ),
        case(
            "lookup table: delete while disabled",
            [delete("lookup_tables", w.lookup_table_off)],
            vec![],
        ),
        // ── processors: every node; a disabled processor still routes, as `null`
        case(
            "processor: insert",
            [insert_processor(x)],
            vec![w.all("processor", x, UPSERT)],
        ),
        case(
            "processor: update",
            [set("processors", w.processor, "name = 'renamed'")],
            vec![w.all("processor", w.processor, UPSERT)],
        ),
        case(
            "processor: disable",
            [set("processors", w.processor, "adm_status = 0")],
            vec![w.all("processor", w.processor, UPSERT)],
        ),
        case(
            "processor: delete",
            [delete("processors", x)],
            vec![w.all("processor", x, DELETE)],
        )
        .given([insert_processor(x)]),
        case(
            "processor: add an input",
            [processor_input(x, w.processor)],
            vec![w.all("processor", x, UPSERT)],
        )
        .given([insert_processor(x)]),
        case(
            "processor: remove an input",
            [format!(
                "DELETE FROM processor_inputs WHERE processor_id = '{}'",
                w.processor_2
            )],
            vec![w.all("processor", w.processor_2, UPSERT)],
        ),
        // ── cluster nodes: every other node; only what a peer connects with
        case(
            "cluster node: insert",
            [insert_node(x)],
            vec![w.all("cluster_node", x, UPSERT)],
        ),
        case(
            "cluster node: metadata",
            [set("cluster_nodes", n2, "metadata = '{\"a\": 1}'")],
            vec![on(n1, "cluster_node", n2, UPSERT)],
        ),
        case(
            "cluster node: op_status",
            [set("cluster_nodes", n2, "op_status = 0")],
            vec![on(n1, "cluster_node", n2, UPSERT)],
        ),
        case(
            "cluster node: heartbeat",
            [set(
                "cluster_nodes",
                n2,
                "last_heartbeat = now() + interval '1 second'",
            )],
            vec![],
        ),
        case(
            "cluster node: delete",
            [delete("cluster_nodes", x)],
            vec![w.all("cluster_node", x, DELETE)],
        )
        .given([insert_node(x)]),
        // ── hosts: the tree to its node, its listener items to every node
        case(
            "host: insert",
            [insert_host(x, Some(n1), 1)],
            vec![on(n1, "host", x, UPSERT)],
        ),
        case("host: insert unassigned", [insert_host(x, None, 1)], vec![]),
        case(
            "host: insert disabled",
            [insert_host(x, Some(n1), 0)],
            vec![],
        ),
        case(
            "host: update",
            [set("hosts", w.host, "name = 'renamed'")],
            vec![on(n1, "host", w.host, UPSERT)],
        ),
        case(
            "host: update while disabled",
            [set("hosts", w.host_off, "name = 'renamed'")],
            vec![],
        ),
        case(
            "host: update unassigned",
            [set("hosts", w.host_free, "name = 'renamed'")],
            vec![],
        ),
        case(
            "host: disable",
            [set("hosts", w.host, "adm_status = 0")],
            vec![w.host_tree(n1, DELETE), w.host_listeners(DELETE)],
        ),
        case(
            "host: enable",
            [set("hosts", w.host_off, "adm_status = 1")],
            vec![
                on(n1, "host", w.host_off, UPSERT),
                on(n1, "host_endpoint", w.endpoint_host_off, UPSERT),
                on(n1, "item", w.item_host_off, UPSERT),
                on(n1, "item", w.listener_item_host_off, UPSERT),
                w.all("listener_endpoint", w.endpoint_host_off, UPSERT),
                w.all("listener_item", w.listener_item_host_off, UPSERT),
            ],
        ),
        case(
            "host: move to another node",
            [set("hosts", w.host, &format!("monitored_by = '{n2}'"))],
            vec![
                w.host_tree(n1, DELETE),
                w.host_tree(n2, UPSERT),
                w.host_listeners(UPSERT),
            ],
        ),
        case(
            "host: assign",
            [set("hosts", w.host_free, &format!("monitored_by = '{n1}'"))],
            vec![
                on(n1, "host", w.host_free, UPSERT),
                on(n1, "host_endpoint", w.endpoint_free, UPSERT),
                on(n1, "item", w.listener_item_free, UPSERT),
                w.all("listener_endpoint", w.endpoint_free, UPSERT),
                w.all("listener_item", w.listener_item_free, UPSERT),
            ],
        ),
        // the listener items carry `monitored_by`, so they are re-read
        case(
            "host: unassign",
            [set("hosts", w.host, "monitored_by = NULL")],
            vec![w.host_tree(n1, DELETE), w.host_listeners(UPSERT)],
        ),
        case(
            "host: move while disabled",
            [set("hosts", w.host_off, &format!("monitored_by = '{n2}'"))],
            vec![],
        ),
        case(
            "host: disable unassigned",
            [set("hosts", w.host_free, "adm_status = 0")],
            vec![
                w.all("listener_endpoint", w.endpoint_free, DELETE),
                w.all("listener_item", w.listener_item_free, DELETE),
            ],
        ),
        case(
            "host: delete",
            [delete("hosts", x)],
            vec![on(n1, "host", x, DELETE)],
        )
        .given([insert_host(x, Some(n1), 1)]),
        case("host: delete while disabled", [delete("hosts", x)], vec![]).given([insert_host(
            x,
            Some(n1),
            0,
        )]),
        // the API's delete: items, templates, endpoints, then the host
        case(
            "host: delete with everything under it",
            [
                format!(
                    "DELETE FROM items USING host_templates \
                     WHERE items.host_template_id = host_templates.host_template_id \
                     AND host_templates.host_id = '{}'",
                    w.host
                ),
                format!("DELETE FROM host_templates WHERE host_id = '{}'", w.host),
                format!("DELETE FROM host_endpoints WHERE host_id = '{}'", w.host),
                delete("hosts", w.host),
            ],
            vec![
                on(n1, "item", w.item, DELETE),
                on(n1, "item", w.listener_item, DELETE),
                on(n1, "item", w.attribute_item, DELETE),
                on(n1, "item", w.child_item, DELETE),
                w.host_listeners(DELETE),
                on(n1, "host_endpoint", w.endpoint, DELETE),
                on(n1, "host", w.host, DELETE),
            ],
        ),
        // ── endpoints
        case(
            "endpoint: insert",
            [insert_endpoint(x, w.host, 1)],
            vec![on(n1, "host_endpoint", x, UPSERT)],
        ),
        case(
            "endpoint: insert disabled",
            [insert_endpoint(x, w.host, 0)],
            vec![],
        ),
        case(
            "endpoint: insert on a disabled host",
            [insert_endpoint(x, w.host_off, 1)],
            vec![],
        ),
        case(
            "endpoint: insert on an unassigned host",
            [insert_endpoint(x, w.host_free, 1)],
            vec![],
        ),
        case(
            "endpoint: update",
            [set("host_endpoints", w.endpoint, "name = 'renamed'")],
            vec![on(n1, "host_endpoint", w.endpoint, UPSERT)],
        ),
        case(
            "endpoint: change address",
            [set(
                "host_endpoints",
                w.endpoint,
                "config = '{\"address\": \"10.0.0.2:161\"}'",
            )],
            vec![
                on(n1, "host_endpoint", w.endpoint, UPSERT),
                w.all("listener_endpoint", w.endpoint, UPSERT),
                w.all("listener_item", w.listener_item, UPSERT),
            ],
        ),
        case(
            "endpoint: disable",
            [set("host_endpoints", w.endpoint, "adm_status = 0")],
            vec![
                w.all("listener_endpoint", w.endpoint, DELETE),
                w.all("listener_item", w.listener_item, DELETE),
                on(n1, "host_endpoint", w.endpoint, DELETE),
                on(n1, "item", w.item, DELETE),
                on(n1, "item", w.listener_item, DELETE),
                on(n1, "item", w.child_item, DELETE),
            ],
        ),
        case(
            "endpoint: enable",
            [set("host_endpoints", w.endpoint_off, "adm_status = 1")],
            vec![
                on(n1, "host_endpoint", w.endpoint_off, UPSERT),
                on(n1, "item", w.item_on_endpoint_off, UPSERT),
            ],
        ),
        case(
            "endpoint: update while disabled",
            [set("host_endpoints", w.endpoint_off, "name = 'renamed'")],
            vec![],
        ),
        case(
            "endpoint: change address on an unassigned host",
            [set(
                "host_endpoints",
                w.endpoint_free,
                "config = '{\"address\": \"10.0.0.2:161\"}'",
            )],
            vec![
                w.all("listener_endpoint", w.endpoint_free, UPSERT),
                w.all("listener_item", w.listener_item_free, UPSERT),
            ],
        ),
        case(
            "endpoint: disable on an unassigned host",
            [set("host_endpoints", w.endpoint_free, "adm_status = 0")],
            vec![
                w.all("listener_endpoint", w.endpoint_free, DELETE),
                w.all("listener_item", w.listener_item_free, DELETE),
            ],
        ),
        case(
            "endpoint: disable on a disabled host",
            [set("host_endpoints", w.endpoint_host_off, "adm_status = 0")],
            vec![],
        ),
        case(
            "endpoint: delete",
            [delete("host_endpoints", x)],
            vec![on(n1, "host_endpoint", x, DELETE)],
        )
        .given([insert_endpoint(x, w.host, 1)]),
        // ── items
        case(
            "item: insert",
            [insert_item(x, w.host_ht, Some(w.endpoint), None, 1)],
            vec![on(n1, "item", x, UPSERT)],
        ),
        case(
            "item: insert disabled",
            [insert_item(x, w.host_ht, Some(w.endpoint), None, 0)],
            vec![],
        ),
        case(
            "item: insert on a disabled endpoint",
            [insert_item(x, w.host_ht, Some(w.endpoint_off), None, 1)],
            vec![],
        ),
        case(
            "item: insert on a disabled host",
            [insert_item(
                x,
                w.host_off_ht,
                Some(w.endpoint_host_off),
                None,
                1,
            )],
            vec![],
        ),
        case(
            "item: insert on an unassigned host",
            [insert_item(
                x,
                w.host_free_ht,
                Some(w.endpoint_free),
                None,
                1,
            )],
            vec![],
        ),
        case(
            "item: update",
            [set("items", w.item, "name = 'renamed'")],
            vec![on(n1, "item", w.item, UPSERT)],
        ),
        case(
            "item: disable",
            [set("items", w.item, "adm_status = 0")],
            vec![on(n1, "item", w.item, DELETE)],
        ),
        case(
            "item: enable",
            [set("items", w.item_off, "adm_status = 1")],
            vec![on(n1, "item", w.item_off, UPSERT)],
        ),
        case(
            "item: update while disabled",
            [set("items", w.item_off, "name = 'renamed'")],
            vec![],
        ),
        case(
            "item: update on a disabled endpoint",
            [set("items", w.item_on_endpoint_off, "name = 'renamed'")],
            vec![],
        ),
        case(
            "item: delete",
            [delete("items", w.item)],
            vec![on(n1, "item", w.item, DELETE)],
        ),
        case(
            "item: delete while disabled",
            [delete("items", w.item_off)],
            vec![],
        ),
        case(
            "item: move to a disabled endpoint",
            [set(
                "items",
                w.item,
                &format!("endpoint_id = '{}'", w.endpoint_off),
            )],
            vec![on(n1, "item", w.item, DELETE)],
        ),
        case(
            "item: move to an enabled endpoint",
            [set(
                "items",
                w.item_on_endpoint_off,
                &format!("endpoint_id = '{}'", w.endpoint),
            )],
            vec![on(n1, "item", w.item_on_endpoint_off, UPSERT)],
        ),
        // the inheriting items are what a node runs
        case(
            "template item: update",
            [set("items", w.template_item, "name = 'renamed'")],
            vec![on(n1, "item", w.child_item, UPSERT)],
        ),
        case(
            "template item: insert",
            [insert_item(x, w.template_ht, None, None, 1)],
            vec![],
        ),
        // ── listener items: to every node, with their endpoint while it has one
        case(
            "listener item: insert the first on an endpoint",
            [insert_item(y, w.host_ht, Some(x), Some(w.listener), 1)],
            vec![
                w.all("listener_endpoint", x, UPSERT),
                w.all("listener_item", y, UPSERT),
                on(n1, "item", y, UPSERT),
            ],
        )
        .given([insert_endpoint(x, w.host, 1)]),
        case(
            "listener item: insert another on an endpoint",
            [insert_item(
                x,
                w.host_ht,
                Some(w.endpoint),
                Some(w.listener),
                1,
            )],
            vec![w.all("listener_item", x, UPSERT), on(n1, "item", x, UPSERT)],
        ),
        case(
            "listener item: insert without an endpoint",
            [insert_item(x, w.host_ht, None, Some(w.listener), 1)],
            vec![w.all("listener_item", x, UPSERT), on(n1, "item", x, UPSERT)],
        ),
        case(
            "listener item: insert on an unassigned host",
            [insert_item(
                x,
                w.host_free_ht,
                Some(w.endpoint_free),
                Some(w.listener),
                1,
            )],
            vec![w.all("listener_item", x, UPSERT)],
        ),
        case(
            "listener item: update",
            [set("items", w.listener_item, "name = 'renamed'")],
            vec![
                w.all("listener_endpoint", w.endpoint, UPSERT),
                w.all("listener_item", w.listener_item, UPSERT),
                on(n1, "item", w.listener_item, UPSERT),
            ],
        ),
        case(
            "listener item: disable the last on an endpoint",
            [set("items", w.listener_item, "adm_status = 0")],
            vec![
                w.all("listener_endpoint", w.endpoint, DELETE),
                w.all("listener_item", w.listener_item, DELETE),
                on(n1, "item", w.listener_item, DELETE),
            ],
        ),
        case(
            "listener item: disable one of two on an endpoint",
            [set("items", w.listener_item, "adm_status = 0")],
            vec![
                w.all("listener_item", w.listener_item, DELETE),
                on(n1, "item", w.listener_item, DELETE),
            ],
        )
        .given([insert_item(
            x,
            w.host_ht,
            Some(w.endpoint),
            Some(w.listener),
            1,
        )]),
        case(
            "listener item: disable without an endpoint",
            [set("items", w.attribute_item, "adm_status = 0")],
            vec![
                w.all("listener_item", w.attribute_item, DELETE),
                on(n1, "item", w.attribute_item, DELETE),
            ],
        ),
        case(
            "listener item: detach from its listener",
            [set("items", w.listener_item, "listener_id = NULL")],
            vec![
                w.all("listener_endpoint", w.endpoint, DELETE),
                w.all("listener_item", w.listener_item, DELETE),
                on(n1, "item", w.listener_item, UPSERT),
            ],
        ),
        case(
            "listener item: change its type",
            [set("items", w.listener_item, "type = 'snmp'")],
            vec![
                w.all("listener_endpoint", w.endpoint, DELETE),
                w.all("listener_item", w.listener_item, DELETE),
                on(n1, "item", w.listener_item, UPSERT),
            ],
        ),
        case(
            "listener item: move to another endpoint",
            [set(
                "items",
                w.listener_item,
                &format!("endpoint_id = '{x}'"),
            )],
            vec![
                w.all("listener_endpoint", w.endpoint, DELETE),
                w.all("listener_endpoint", x, UPSERT),
                w.all("listener_item", w.listener_item, UPSERT),
                on(n1, "item", w.listener_item, UPSERT),
            ],
        )
        .given([insert_endpoint(x, w.host, 1)]),
        case(
            "listener item: move to a disabled endpoint",
            [set(
                "items",
                w.listener_item,
                &format!("endpoint_id = '{}'", w.endpoint_off),
            )],
            vec![
                w.all("listener_endpoint", w.endpoint, DELETE),
                w.all("listener_item", w.listener_item, DELETE),
                on(n1, "item", w.listener_item, DELETE),
            ],
        ),
        case(
            "listener item: delete",
            [delete("items", w.listener_item)],
            vec![
                w.all("listener_endpoint", w.endpoint, DELETE),
                w.all("listener_item", w.listener_item, DELETE),
                on(n1, "item", w.listener_item, DELETE),
            ],
        ),
        // ── item processors: the item, and every item inheriting from it, re-read
        case(
            "item processor: insert",
            [item_processor(w.item, w.processor)],
            vec![on(n1, "item", w.item, UPSERT)],
        ),
        case(
            "item processor: delete",
            [format!(
                "DELETE FROM item_processors WHERE item_id = '{}'",
                w.item
            )],
            vec![on(n1, "item", w.item, UPSERT)],
        )
        .given([item_processor(w.item, w.processor)]),
        case(
            "item processor: insert on a listener item",
            [item_processor(w.listener_item, w.processor)],
            vec![
                w.all("listener_item", w.listener_item, UPSERT),
                on(n1, "item", w.listener_item, UPSERT),
            ],
        ),
        case(
            "item processor: insert on a template item",
            [item_processor(w.template_item, w.processor)],
            vec![on(n1, "item", w.child_item, UPSERT)],
        ),
        case(
            "item processor: insert on a disabled host",
            [item_processor(w.item_host_off, w.processor)],
            vec![],
        ),
    ]
}

fn item_processor(item: Uuid, processor: Uuid) -> String {
    format!("INSERT INTO item_processors (item_id, processor_id) VALUES ('{item}', '{processor}')")
}

/// Every resource a trigger may write. Listing one here is what a new variant
/// needs to compile.
fn every_resource() -> Vec<ClusterLogResource> {
    use ClusterLogResource::*;
    let all = vec![
        Host,
        HostEndpoint,
        Item,
        Listener,
        ListenerItem,
        ListenerEndpoint,
        LookupTable,
        Processor,
        Sink,
        ClusterNode,
    ];
    for r in &all {
        match r {
            Host | HostEndpoint | Item | Listener | ListenerItem | ListenerEndpoint
            | LookupTable | Processor | Sink | ClusterNode => {}
        }
    }
    all
}

async fn logged(tx: &Transaction<'_>) -> Result<Vec<Row>> {
    let rows = tx
        .query(
            "SELECT node_id, resource, resource_id, op FROM cluster_logs",
            &[],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| Row {
            node: r.get(0),
            resource: r.get(1),
            id: r.get(2),
            op: r.get(3),
        })
        .collect())
}

/// `node`'s view, as the watcher reads it: the latest op per resource.
async fn view(tx: &Transaction<'_>, node: Uuid) -> Result<BTreeMap<(String, Uuid), i16>> {
    let mut params = query::ClusterLogParams::new(node);
    params.with_columns(vec![
        ClusterLogs::Resource,
        ClusterLogs::ResourceId,
        ClusterLogs::Op,
    ]);
    let (sql, values) = query::cluster_logs(params).build_postgres(PostgresQueryBuilder);
    let rows = tx.query(&sql, &values.as_params()).await?;
    Ok(rows
        .iter()
        .map(|r| ((r.get(0), r.get(1)), r.get(2)))
        .collect())
}

/// What `case` got wrong, if anything.
async fn check(
    tx: &Transaction<'_>,
    w: &World,
    case: &Case,
    seen: &mut BTreeSet<String>,
) -> Result<Vec<String>> {
    for sql in &case.setup {
        tx.batch_execute(sql)
            .await
            .with_context(|| format!("setup: {sql}"))?;
    }
    tx.batch_execute("DELETE FROM cluster_logs").await?;
    for sql in &case.action {
        tx.batch_execute(sql)
            .await
            .with_context(|| format!("action: {sql}"))?;
    }
    let actual = logged(tx).await?;
    let mut problems = Vec::new();
    for row in &actual {
        if ClusterLogResource::from_str(&row.resource).is_err() {
            problems.push(format!("no ClusterLogResource for {:?}", row.resource));
        }
        seen.insert(row.resource.clone());
    }

    let mut counts: BTreeMap<&Row, i32> = BTreeMap::new();
    for row in &case.expect {
        *counts.entry(row).or_default() += 1;
    }
    for row in &actual {
        *counts.entry(row).or_default() -= 1;
    }
    for (row, n) in counts {
        let label = if n > 0 { "missing:   " } else { "unexpected:" };
        for _ in 0..n.abs() {
            problems.push(format!("{label} {}", w.show(row)));
        }
    }
    if !problems.is_empty() {
        return Ok(problems);
    }

    for node in [w.n1, w.n2] {
        let mut expected = BTreeMap::new();
        for row in case.expect.iter().filter(|r| r.node == node) {
            expected.insert((row.resource.clone(), row.id), row.op);
        }
        let got = view(tx, node).await?;
        if got != expected {
            let show = |v: &BTreeMap<(String, Uuid), i16>| {
                v.iter()
                    .map(|((resource, id), op)| {
                        w.show(&Row {
                            node,
                            resource: resource.clone(),
                            id: *id,
                            op: *op,
                        })
                    })
                    .collect::<Vec<_>>()
            };
            problems.push(format!(
                "{} reads {:?}, expected {:?}",
                w.name(node),
                show(&got),
                show(&expected)
            ));
        }
    }
    Ok(problems)
}

/// The trigger matrix: config table × change → the cluster log rows written.
/// Would have caught N3 and N35.
#[tokio::test]
async fn every_config_change_logs_what_each_node_needs() -> Result<()> {
    setup::setup_logging();
    let (_db, db_pool) = setup::setup_db().await;
    let w = World::new();
    let mut conn = db_pool.get().await?;
    for sql in w.sql() {
        conn.batch_execute(&sql)
            .await
            .with_context(|| format!("world: {sql}"))?;
    }

    let cases = cases(&w);
    let mut failures = Vec::new();
    let mut seen = BTreeSet::new();
    for case in &cases {
        let tx = conn.transaction().await?;
        let problems = match check(&tx, &w, case, &mut seen).await {
            Ok(problems) => problems,
            Err(e) => vec![format!("{e:#}")],
        };
        tx.rollback().await?;
        if !problems.is_empty() {
            failures.push(format!("{}\n    {}", case.name, problems.join("\n    ")));
        }
    }

    for resource in every_resource() {
        let s = resource.as_str();
        if ClusterLogResource::from_str(s).ok() != Some(resource) {
            failures.push(format!("{s:?} does not parse back to {resource:?}"));
        }
        if !seen.contains(s) {
            failures.push(format!("no case logs a {s:?} row"));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n\n")
    );
    Ok(())
}
