---
description: Add nodes to a Patrolli cluster, so the leader shares the hosts out and moves them when a node stops.
---

# Clustering

A cluster is a group of Patrolli nodes that share one PostgreSQL database. One node is the leader. The leader gives each host to one node, and that node collects the data of the host. When a node stops, the leader moves its hosts to the nodes that still run.

This guide adds `node-2` and `node-3` to the deployment from [Single Node with Backend](./single-node-with-backend), so the cluster has three nodes. Only `node-1` serves the REST API, because one API node is enough for the web interface.

## Before you begin

You need the following:

- The deployment from [Single Node with Backend](./single-node-with-backend), running. Run every command of this guide in its `patrolli` directory.

## 1. Add two nodes

Create a directory for each new node:

```sh
mkdir node-2 node-3
```

Give each new node the secret keys of `node-1`:

```sh
grep ^SECRET_ node-1/.env > node-2/.env
grep ^SECRET_ node-1/.env > node-3/.env
```

Create the Compose files and the configuration files:

::: code-group

```yaml [node-2/compose.yaml]
name: patrolli-node-2

services:
    patrolli-2:
        image: patrolli-core
        container_name: patrolli-2
        restart: unless-stopped
        environment:
            PATROLLI_SECRET_KEYS: ${SECRET_KEYS:?}
            PATROLLI_SECRET_ACTIVE_KEY: ${SECRET_ACTIVE_KEY:?}
        volumes:
            - ./config.yaml:/config.yaml:ro
            - data:/var/lib/patrolli
        networks:
            - patrolli

volumes:
    data:

networks:
    patrolli:
        external: true
```

```yaml [node-3/compose.yaml]
name: patrolli-node-3

services:
    patrolli-3:
        image: patrolli-core
        container_name: patrolli-3
        restart: unless-stopped
        environment:
            PATROLLI_SECRET_KEYS: ${SECRET_KEYS:?}
            PATROLLI_SECRET_ACTIVE_KEY: ${SECRET_ACTIVE_KEY:?}
        volumes:
            - ./config.yaml:/config.yaml:ro
            - data:/var/lib/patrolli
        networks:
            - patrolli

volumes:
    data:

networks:
    patrolli:
        external: true
```

:::

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

```yaml [node-3/config.yaml]
global:
    backend:
        db_url: postgresql://patrolli:change-me@patrolli-postgres:5432/patrolli
        cluster:
            node_id: 01a16349-af20-7fe4-a372-f84d47bef975
            node_name: node-3
            metadata:
                advertise_address: patrolli-3:4636
```

:::

Compare the new files with the files of `node-1`:

- **Same `db_url`:** the new nodes use the database of `node-1`. A shared database is what puts nodes in the same cluster.
- **Own `node_id`, `node_name`, and `advertise_address`:** these differ on every node. The other nodes reach a node at its `advertise_address`.
- **No `api` block:** the new nodes do not serve the REST API, so they need no `PATROLLI_JWT_SECRET`.
- **Same secret keys:** every node of a cluster needs the same `PATROLLI_SECRET_KEYS` and `PATROLLI_SECRET_ACTIVE_KEY`. A node without the `api` block needs them too.
- **No `PATROLLI_ADMIN_USER` or `PATROLLI_ADMIN_PASSWORD`:** `node-1` created the first user already.

Start the new nodes:

```sh
docker compose -f node-2/compose.yaml up -d
docker compose -f node-3/compose.yaml up -d
```

Check that `node-2` joined the cluster. If the command prints nothing, wait a few seconds and run it again:

```sh
docker logs patrolli-2 2>&1 | grep "node state changed"
```

The output looks like this:

```text
2026-10-22T11:08:49.756126153+00:00  INFO run{node_id=01a16349-af1d-7b55-9abd-fe787a66bc3c node=node-2}: patrolli::cluster: node state changed from=LostConnection to=Main
```

`LostConnection` is the state of a node before it first reaches the database. `Main` means that the node runs and takes hosts, but is not the leader. Run the same command for `patrolli-3`.

## 2. Check the cluster

Open `http://192.168.1.10:3000` and sign in as `admin`. Click **Cluster Monitoring**. The page lists the three nodes as **Active**. `node-1` is the leader, because it started first.

![Cluster Monitoring lists node-1, node-2, and node-3 as active, with node-1 as leader](./assets/clustering/three-nodes.png)

## 3. Add hosts

Create three hosts to see how the leader shares them out:

1. **Open the host form**

    Click **Configuration**, then **Hosts**, then **Create Host**.

2. **Create a host**

    Enter `router-1` as the **Name**, and click **Save**.

    ![The Create Host form with router-1 as the name](./assets/clustering/create-host.png)

3. **Create two more hosts**

    Create `router-2` and `router-3` in the same way.

4. **Check where the hosts are**

    Reload the **Hosts** page. The **Cluster Node** column shows the node that collects each host. The leader puts each host on a different node. The leader places new hosts every five seconds, so if a host shows no node yet, wait a few seconds and reload again.

    ![The Hosts page with router-1, router-2, and router-3 on three different nodes](./assets/clustering/hosts-spread.png)

## 4. Stop a node

Stop `node-3`:

```sh
docker stop patrolli-3
```

Wait about 15 seconds, and reload the **Hosts** page. The host of `node-3` now runs on one of the other two nodes. In **Cluster Monitoring**, click **Refresh** to see the change. The page first shows `node-3` as **Inactive**, and then no longer lists it.

![The Hosts page after node-3 stopped: no host is on node-3](./assets/clustering/host-moved.png)

## 5. Start the node again

Start `node-3`:

```sh
docker start patrolli-3
```

After a few seconds, click **Refresh** in **Cluster Monitoring**. The page lists `node-3` as **Active** again.

## 6. Stop the leader

When the leader stops, another node becomes the leader. Stop `node-1`:

```sh
docker stop patrolli-1
```

Find the new leader in the logs of the other two nodes. If the command prints nothing, wait a few seconds and run it again:

```sh
docker logs patrolli-2 2>&1 | grep "node state changed" | grep Leader || docker logs patrolli-3 2>&1 | grep "node state changed" | grep Leader
```

Either `node-2` or `node-3` can win. The output looks like this:

```text
2026-10-22T11:10:57.547921232+00:00  INFO run{node_id=01a16349-af20-7fe4-a372-f84d47bef975 node=node-3}: patrolli::cluster: node state changed from=Main to=Leader
```

The new leader also moves the hosts of `node-1` to the nodes that still run. While `node-1` is down, the web interface cannot reach the cluster, because `node-1` is the only node with the REST API.

Start `node-1` again:

```sh
docker start patrolli-1
```

Check that `node-1` joined as `Main`. If the command prints nothing, wait a few seconds and run it again:

```sh
docker logs patrolli-1 2>&1 | grep "node state changed" | grep Main
```

```text
2026-10-22T11:20:26.927673458+00:00  INFO run{node_id=01a16349-af19-740a-a6c0-3465a817afdc node=node-1}: patrolli::cluster: node state changed from=LostConnection to=Main
```

The leader role stays with the node that took it over.

## Remove the deployment

To delete the two new nodes and their data, run:

```sh
docker compose -f node-3/compose.yaml down -v
docker compose -f node-2/compose.yaml down -v
```

Then follow [Remove the deployment](./single-node-with-backend#remove-the-deployment) in the Single Node with Backend guide to delete the rest.

## Next steps

- Run several separate clusters and manage them from one web interface in [Multiple Cluster](./multiple-cluster). That guide starts on an empty host, so remove this deployment first.
