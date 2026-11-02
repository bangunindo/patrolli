---
description: The ways to deploy Patrolli, from one node with a YAML file to several clusters behind one web interface, and the roles that a node takes in a cluster.
---

# Architecture

Patrolli runs as one node or as many. Each architecture on this page adds one thing to the architecture before it, and each diagram highlights the part that changed. Start with the simplest architecture that covers what you need.

## Single node without a backend

![Devices send data to one Patrolli node, which reads its configuration from config.yaml and delivers the data to its destinations.](./assets/architecture/single-node-without-backend-light.svg){.light-only}
![Devices send data to one Patrolli node, which reads its configuration from config.yaml and delivers the data to its destinations.](./assets/architecture/single-node-without-backend-dark.svg){.dark-only}

- One node reads its whole configuration from a YAML file. It needs no database.
- To change the configuration, you edit the file and restart the node. This architecture has no REST API and no web interface.

To set it up, follow [Single Node without Backend](../getting-started/deployment/single-node-without-backend).

## Single node with a backend

![The single node now keeps its configuration in PostgreSQL, and the web interface changes it through the REST API.](./assets/architecture/single-node-with-backend-light.svg){.light-only}
![The single node now keeps its configuration in PostgreSQL, and the web interface changes it through the REST API.](./assets/architecture/single-node-with-backend-dark.svg){.dark-only}

- The node keeps its configuration in PostgreSQL. You change the configuration in the web interface or through the REST API, and the node applies each change without a restart.
- Every architecture below keeps this database and this web interface.

To set it up, follow [Single Node with Backend](../getting-started/deployment/single-node-with-backend).

## Cluster

![node-2 and node-3 join node-1. The three nodes share one PostgreSQL database, and node-1 is the leader.](./assets/architecture/cluster-light.svg){.light-only}
![node-2 and node-3 join node-1. The three nodes share one PostgreSQL database, and node-1 is the leader.](./assets/architecture/cluster-dark.svg){.dark-only}

- Several nodes share one PostgreSQL database. The leader gives each host to one node, and only that node collects the data of the host.
- When a node stops, the leader moves its hosts to the nodes that still run. With the default heartbeat settings, the hosts move about 10 seconds after the node stops.

To set it up, follow [Clustering](../getting-started/deployment/clustering).

## Cluster with a spare node

![With max_active_nodes set to 2, node-3 carries no hosts and waits as a spare.](./assets/architecture/cluster-spare-light.svg){.light-only}
![With max_active_nodes set to 2, node-3 carries no hosts and waits as a spare.](./assets/architecture/cluster-spare-dark.svg){.dark-only}

- `max_active_nodes` limits how many nodes carry hosts. The other nodes run as spares and carry no hosts.
- When a node stops, all its hosts move to one spare, so the load on the other nodes does not change. Use a spare when each node has room only for its own hosts.

To set it up, follow [Clustering](../getting-started/deployment/clustering), then [set `max_active_nodes`](#spare).

## Cluster with standby processes

![node-1 and node-2 each get a standby process with the same node_id, which waits under the active process.](./assets/architecture/cluster-standby-light.svg){.light-only}
![node-1 and node-2 each get a standby process with the same node_id, which waits under the active process.](./assets/architecture/cluster-standby-dark.svg){.dark-only}

- A node runs a second Patrolli process with the same `node_id`, and the second process waits as a standby. When the first process exits, the standby takes over the node in less than a second.
- The hosts do not move, because the node stays the same. Run the standby on another machine, so that one machine failure does not stop both processes.

To set it up, follow [Clustering](../getting-started/deployment/clustering), then [add a standby](#standby).

## Multiple clusters

![The cluster becomes one of several. Each cluster has its own nodes and its own PostgreSQL database, and one web interface reaches all of them.](./assets/architecture/multiple-clusters-light.svg){.light-only}
![The cluster becomes one of several. Each cluster has its own nodes and its own PostgreSQL database, and one web interface reaches all of them.](./assets/architecture/multiple-clusters-dark.svg){.dark-only}

- Each cluster has its own nodes, its own database, and its own users.
- One web interface lists every cluster, and you switch clusters there.
- Multiple clusters are useful when you have multiple machines that cannot reach each other.

To set it up, follow [Multiple Cluster](../getting-started/deployment/multiple-cluster). To sign in to every cluster with one account, follow [Authentication](../getting-started/deployment/authentication).

## Node roles

Every Patrolli process in a cluster has one role. A node is one `node_id`, and one node can run more than one process.

![node_id A runs a leader process and a standby process. node_id B runs a main process. node_id C runs a spare process with no hosts.](./assets/architecture/node-roles-light.svg){.light-only}
![node_id A runs a leader process and a standby process. node_id B runs a main process. node_id C runs a spare process with no hosts.](./assets/architecture/node-roles-dark.svg){.dark-only}

| Role    | What it does                                                                                                                          |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| Leader  | Carries hosts like a main node. It also gives each host to a node and moves the hosts of a node that stops. A cluster has one leader. |
| Main    | Holds the lock of its `node_id` and collects the data of its hosts.                                                                   |
| Spare   | A main node that `max_active_nodes` keeps empty. It takes all the hosts of a node that stops.                                         |
| Standby | Waits for the lock of a `node_id` that another process holds. It takes over that node when the other process stops.                   |

A standby and a spare can both cover the same failure. The way the process stops decides which one takes over:

- **The process exits.** This happens after a crash or a `docker stop`, for example. The database connection of the process closes, and the standby takes over the node. The hosts do not move.
- **The machine stops or loses its network.** PostgreSQL keeps the old connection open, and with it the lock of the node, so the standby cannot take over. About 10 seconds later, the leader moves the hosts to the spare. The standby takes over the empty node only after PostgreSQL closes the old connection.

### Spare

A spare needs no setting of its own. Start one more node than `max_active_nodes` allows. The leader reads `max_active_nodes` from the database, not from the YAML file, so set it with SQL:

```sql
INSERT INTO configs (key, val, last_updated)
VALUES ('cluster', '{"max_active_nodes": 2}', now())
ON CONFLICT (key) DO UPDATE SET val = EXCLUDED.val, last_updated = now();
```

The default value `0` lets every node carry hosts, so the cluster has no spare.

### Standby

A standby is a copy of a node with the same `node_id`, a new `node_name`, and its own `advertise_address`. This example adds a standby for `node-2` from the [Clustering](../getting-started/deployment/clustering) guide:

::: code-group

```yaml [node-2/config.yaml]
global:
    backend:
        db_url: postgresql://patrolli:change-me@patrolli-postgres:5432/patrolli
        cluster:
            node_id: 01a16349-af1d-7b55-9abd-fe787a66bc3c
            node_name: node-2
            metadata:
                advertise_address: patrolli-2:4636
```

```yaml [node-2-standby/config.yaml]
global:
    backend:
        db_url: postgresql://patrolli:change-me@patrolli-postgres:5432/patrolli
        cluster:
            node_id: 01a16349-af1d-7b55-9abd-fe787a66bc3c
            node_name: node-2-standby
            metadata:
                advertise_address: patrolli-2-standby:4636
```

:::

Two processes with the same `node_id` and the same `node_name` cannot run together. The second process stops at startup.
