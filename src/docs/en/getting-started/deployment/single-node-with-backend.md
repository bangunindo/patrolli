---
description: Run one Patrolli node with a PostgreSQL database and the web interface, and change the configuration without a restart.
---

# Single Node with Backend

This guide runs one Patrolli node with a PostgreSQL database, the REST API, and the web interface. The node keeps its configuration in the database. You change it in the web interface or through the REST API, and the node applies the change without a restart.

At the end of the guide, these containers run on the host:

- **`patrolli-postgres`:** the PostgreSQL server that holds the configuration.
- **`patrolli-1`:** the Patrolli node. It also serves the REST API.
- **`patrolli-web`:** the web interface, published on port `3000`.

With a backend, a node always belongs to a cluster, even when it is the only node. To add more nodes later, see [Clustering](./clustering).

## Before you begin

You need the following:

- A Linux host with Docker Engine and the Docker Compose plugin.
- The `patrolli-core` and `patrolli-web` images on that host.
- `openssl` on the host, to create a secret.
- The IP address of the host. This guide uses `192.168.1.10`. Replace it with the IP address of your host.

::: warning
The passwords in this guide are examples. Replace them with your own values before you use the deployment for anything other than a test.

The guides share container names and the network `patrolli`. If the host already runs another deployment from these guides, remove it first.
:::

## 1. Start PostgreSQL

All containers in this guide join one Docker network, so they reach each other by container name. Create a directory for each part of the deployment, and create the network:

```sh
mkdir patrolli && cd patrolli
mkdir postgres node-1 web
docker network create patrolli
```

Create the Compose file for PostgreSQL:

::: code-group

```yaml [postgres/compose.yaml]
name: patrolli-postgres

services:
    patrolli-postgres:
        image: postgres:18
        container_name: patrolli-postgres
        restart: unless-stopped
        environment:
            POSTGRES_USER: patrolli
            POSTGRES_PASSWORD: change-me
        volumes:
            - data:/var/lib/postgresql
        healthcheck:
            test: ["CMD", "pg_isready", "-U", "patrolli"]
            interval: 5s
            retries: 10
        networks:
            - patrolli

volumes:
    data:

networks:
    patrolli:
        external: true
```

:::

Start PostgreSQL. With `--wait`, the command returns after the health check passes:

```sh
docker compose -f postgres/compose.yaml up -d --wait
```

At the first start, PostgreSQL creates a database with the name of `POSTGRES_USER`, here `patrolli`. The node uses that database.

## 2. Start the node

The node directory holds three files:

- **`.env`:** the secret that signs the login tokens, and the key that encrypts stored secrets.
- **`compose.yaml`:** runs the node and sets the first admin user.
- **`config.yaml`:** connects the node to the database and turns on the REST API.

Create a random secret and a random secret key:

```sh
echo "JWT_SECRET=$(openssl rand -hex 32)" > node-1/.env
echo "SECRET_KEYS=k1:$(openssl rand -base64 32)" >> node-1/.env
echo "SECRET_ACTIVE_KEY=k1" >> node-1/.env
```

Create the Compose file and the configuration file:

::: code-group

```yaml [node-1/compose.yaml]
name: patrolli-node-1

services:
    patrolli-1:
        image: patrolli-core
        container_name: patrolli-1
        restart: unless-stopped
        environment:
            PATROLLI_ADMIN_USER: admin
            PATROLLI_ADMIN_PASSWORD: change-me
            PATROLLI_JWT_SECRET: ${JWT_SECRET:?}
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

```yaml [node-1/config.yaml]
global:
    backend:
        db_url: postgresql://patrolli:change-me@patrolli-postgres:5432/patrolli
        cluster:
            node_id: 01a16349-af19-740a-a6c0-3465a817afdc
            node_name: node-1
            metadata:
                advertise_address: patrolli-1:4636
        api: {}
```

:::

With a `backend` block, the node reads only the `global` part from `config.yaml`. It keeps everything else in the database: listeners, hosts, items, processors, and sinks.

- **`PATROLLI_ADMIN_USER` and `PATROLLI_ADMIN_PASSWORD`:** create the first user when the database has no users. After that, the node ignores them.
- **`PATROLLI_JWT_SECRET`:** signs the login tokens. Compose takes the value from `JWT_SECRET` in the `.env` file next to `compose.yaml`, and stops with an error when it is missing. The secret must be at least 32 bytes long.
- **`PATROLLI_SECRET_KEYS`:** the keys that encrypt the passwords, passphrases, and tokens that sinks, listeners, and endpoints keep in the database. Write each key as `<id>:<secret>`, and separate the keys with commas. An id has 1 to 20 letters, digits, `-`, or `_`. A secret is any text of at least 16 bytes without commas. A long random value is safest, and `openssl rand -base64 32` is one way to make one.
- **`PATROLLI_SECRET_ACTIVE_KEY`:** the id of the key that encrypts new values. The id must be in `PATROLLI_SECRET_KEYS`. Every node of a cluster needs the same values for both variables. A node without them does not start.

    For example, a new deployment uses one key, `k1`, as the active key:

    ```yaml
    environment:
        PATROLLI_SECRET_KEYS: k1:OaEOl7gaUDcazM+JXYwrDfkfq56kp69Id5C47zsEtsM=
        PATROLLI_SECRET_ACTIVE_KEY: k1
    ```

    ::: warning
    If you lose the keys, you lose every stored secret. Keep a copy of the keys outside the node.
    :::

- **`db_url`:** the database that holds the configuration.
- **`node_id`:** a UUID that identifies the node in its cluster.
- **`advertise_address`:** the address that other nodes of the cluster use to reach this node. Port `4636` is the default port for traffic between nodes.
- **`api`:** turns on the REST API on port `8080`. An empty block uses the defaults.

Start the node:

```sh
docker compose -f node-1/compose.yaml up -d
```

Check that the node started its REST API. A node needs a few seconds to start, so if the command prints nothing, wait and run it again:

```sh
docker logs patrolli-1 2>&1 | grep "starting API server"
```

The output looks like this:

```text
2026-10-22T11:07:09.953039663+00:00  INFO patrolli: starting API server addr=0.0.0.0:8080 tls=false
```

## 3. Start the web interface

The web interface reads the address of the REST API from `config-fe.yaml`. Create the Compose file and the configuration file:

::: code-group

```yaml [web/compose.yaml]
name: patrolli-web

services:
    patrolli-web:
        image: patrolli-web
        container_name: patrolli-web
        restart: unless-stopped
        ports:
            - "3000:3000"
        volumes:
            - ./config-fe.yaml:/app/config-fe.yaml:ro
        networks:
            - patrolli

networks:
    patrolli:
        external: true
```

```yaml [web/config-fe.yaml]
clusters:
    - name: Patrolli
      url: http://patrolli-1:8080
```

:::

The `patrolli-web` container reaches the node by its container name, because both are on the same Docker network.

Start the web interface:

```sh
docker compose -f web/compose.yaml up -d
```

## 4. Sign in

1. **Open the web interface**

    Open `http://192.168.1.10:3000` in a browser. The **Cluster** list shows **Patrolli**, the name from `config-fe.yaml`.

    ![The sign-in page with Patrolli as the cluster](./assets/single-node-with-backend/sign-in.png)

2. **Sign in**

    Enter the username `admin` and the password `change-me`, and click **Sign in**.

3. **Check the node**

    Click **Cluster Monitoring**. The page lists `node-1` as **Active** and as the leader of the cluster.

    ![Cluster Monitoring shows node-1 as active and leader](./assets/single-node-with-backend/cluster-monitoring.png)

The node now applies every change that you save in the web interface. To build a pipeline from hosts, items, processors, and sinks, see [Creating Your First Pipeline](../usage/creating-your-first-pipeline).

## Troubleshooting

1. **`required variable JWT_SECRET is missing a value`**

    Compose cannot find the secret. Create `node-1/.env` as shown in [Start the node](#_2-start-the-node), and start the node again.

2. **Failed to communicate with cluster Patrolli**

    The web interface cannot reach the REST API of the node. Check that `patrolli-1` runs with `docker ps --filter name=patrolli-1`. Then read the errors in the web interface log with `docker logs patrolli-web 2>&1 | grep ERROR`. A line such as `getaddrinfo ENOTFOUND patrolli-1` means that the name in `url` does not resolve from the `patrolli-web` container.

## Remove the deployment

To delete the containers, the network, and all data of this guide, run:

```sh
docker compose -f web/compose.yaml down
docker compose -f node-1/compose.yaml down -v
docker compose -f postgres/compose.yaml down -v
docker network rm patrolli
```

## Next steps

- Add more nodes to the cluster in [Clustering](./clustering).
- Replace the secret keys in [Rotate Secret Keys](./rotate-secret-keys).
- Build a pipeline in [Creating Your First Pipeline](../usage/creating-your-first-pipeline).
