---
description: Run three Patrolli clusters on one Docker host and manage all of them from one web interface.
---

# Multiple Cluster

This guide runs three Patrolli clusters and one web interface on a single Docker host. Each cluster has its own PostgreSQL database and its own users. The web interface keeps a list of the clusters, and you pick a cluster when you sign in.

At the end of the guide, these containers run on the host:

- **`patrolli-postgres`:** one PostgreSQL server with a separate database for each cluster.
- **`patrolli-a-1`, `patrolli-b-1`, `patrolli-c-1`:** one Patrolli node for each cluster. Each node serves the REST API of its cluster.
- **`patrolli-web`:** the web interface, published on port `3000`.

Each cluster in this guide has one node. To learn how the nodes of one cluster share the work, see [Clustering](./clustering).

## Before you begin

You need the following:

- A Linux host with Docker Engine and the Docker Compose plugin.
- The `patrolli-core` and `patrolli-web` images on that host.
- `openssl` on the host, to create secrets.
- The IP address of the host. This guide uses `192.168.1.10`. Replace it with the IP address of your host.

::: warning
The passwords in this guide are examples. Replace them with your own values before you use the deployment for anything other than a test.

The guides share container names and the network `patrolli`. If the host already runs another deployment from these guides, remove it first.
:::

## 1. Start PostgreSQL

All containers in this guide join one Docker network, so they reach each other by container name. Create a directory for each part of the deployment, and create the network:

```sh
mkdir patrolli && cd patrolli
mkdir postgres cluster-a cluster-b cluster-c web
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

Create one database for each cluster:

```sh
docker exec patrolli-postgres psql -U patrolli -c "CREATE DATABASE patrolli_a;"
docker exec patrolli-postgres psql -U patrolli -c "CREATE DATABASE patrolli_b;"
docker exec patrolli-postgres psql -U patrolli -c "CREATE DATABASE patrolli_c;"
```

Each command prints `CREATE DATABASE`.

## 2. Start the clusters

Each cluster has three files in its own directory:

- **`.env`:** the secret that signs the login tokens of the cluster, and the key that encrypts the secrets that the cluster stores.
- **`compose.yaml`:** runs the Patrolli node and sets the first admin user.
- **`config.yaml`:** connects the node to the database of its cluster and turns on the REST API.

Create a random secret and a random secret key for each cluster:

```sh
echo "JWT_SECRET=$(openssl rand -hex 32)" > cluster-a/.env
echo "SECRET_KEYS=k1:$(openssl rand -base64 32)" >> cluster-a/.env
echo "SECRET_ACTIVE_KEY=k1" >> cluster-a/.env
echo "JWT_SECRET=$(openssl rand -hex 32)" > cluster-b/.env
echo "SECRET_KEYS=k1:$(openssl rand -base64 32)" >> cluster-b/.env
echo "SECRET_ACTIVE_KEY=k1" >> cluster-b/.env
echo "JWT_SECRET=$(openssl rand -hex 32)" > cluster-c/.env
echo "SECRET_KEYS=k1:$(openssl rand -base64 32)" >> cluster-c/.env
echo "SECRET_ACTIVE_KEY=k1" >> cluster-c/.env
```

The Compose files and the configuration files of the three clusters differ only in names, the database, and the node ID. Create the Compose files:

::: code-group

```yaml [cluster-a/compose.yaml]
name: patrolli-cluster-a

services:
    patrolli-a-1:
        image: patrolli-core
        container_name: patrolli-a-1
        restart: unless-stopped
        environment:
            PATROLLI_ADMIN_USER: admin
            PATROLLI_ADMIN_PASSWORD: change-me
            PATROLLI_ADMIN_ALLOW_SSO: "true"
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

```yaml [cluster-b/compose.yaml]
name: patrolli-cluster-b

services:
    patrolli-b-1:
        image: patrolli-core
        container_name: patrolli-b-1
        restart: unless-stopped
        environment:
            PATROLLI_ADMIN_USER: admin
            PATROLLI_ADMIN_PASSWORD: change-me
            PATROLLI_ADMIN_ALLOW_SSO: "true"
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

```yaml [cluster-c/compose.yaml]
name: patrolli-cluster-c

services:
    patrolli-c-1:
        image: patrolli-core
        container_name: patrolli-c-1
        restart: unless-stopped
        environment:
            PATROLLI_ADMIN_USER: admin
            PATROLLI_ADMIN_PASSWORD: change-me
            PATROLLI_ADMIN_ALLOW_SSO: "true"
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

:::

Create the configuration files:

::: code-group

```yaml [cluster-a/config.yaml]
global:
    backend:
        db_url: postgresql://patrolli:change-me@patrolli-postgres:5432/patrolli_a
        cluster:
            node_id: 01a162d5-72de-7b62-bc3c-848dd2ea0567
            node_name: node-1
            metadata:
                advertise_address: patrolli-a-1:4636
        api:
            auth:
                local:
                    jwt_issuer: patrolli-a
```

```yaml [cluster-b/config.yaml]
global:
    backend:
        db_url: postgresql://patrolli:change-me@patrolli-postgres:5432/patrolli_b
        cluster:
            node_id: 01a162d5-72e1-7788-8aee-af7caa86f1bd
            node_name: node-1
            metadata:
                advertise_address: patrolli-b-1:4636
        api:
            auth:
                local:
                    jwt_issuer: patrolli-b
```

```yaml [cluster-c/config.yaml]
global:
    backend:
        db_url: postgresql://patrolli:change-me@patrolli-postgres:5432/patrolli_c
        cluster:
            node_id: 01a162d5-72e3-7d54-9d20-1306d2c9189e
            node_name: node-1
            metadata:
                advertise_address: patrolli-c-1:4636
        api:
            auth:
                local:
                    jwt_issuer: patrolli-c
```

:::

These settings matter for a multi-cluster setup:

- **`PATROLLI_ADMIN_USER` and `PATROLLI_ADMIN_PASSWORD`:** create the first user when the database of the cluster has no users. After that, the node ignores them.
- **`PATROLLI_ADMIN_ALLOW_SSO`:** lets the first user sign in with single sign-on. The [Authentication](./authentication) guide uses it.
- **`PATROLLI_JWT_SECRET`:** signs the login tokens of the cluster. Compose takes the value from `JWT_SECRET` in the `.env` file next to `compose.yaml`, and stops with an error when it is missing. The secret must be at least 32 bytes long, and each cluster needs its own.
- **`PATROLLI_SECRET_KEYS` and `PATROLLI_SECRET_ACTIVE_KEY`:** encrypt the passwords, passphrases, and tokens that the cluster keeps in its database. Compose takes the values from `SECRET_KEYS` and `SECRET_ACTIVE_KEY` in the same `.env` file. Each cluster can have its own key. A long random value is safest, and the commands above make one with `openssl rand -base64 32`.
- **`db_url`:** the database of the cluster. Two clusters must never share a database.
- **`node_id`:** a UUID that identifies the node in its cluster.
- **`advertise_address`:** the address that other nodes of the same cluster use to reach this node. Port `4636` is the default port for traffic between nodes.
- **`api`:** turns on the REST API on port `8080`. The web interface sends its requests to this API.
- **`jwt_issuer`:** the name that the cluster writes into its login tokens. A cluster rejects login tokens that carry the name of another cluster.

Start the clusters:

```sh
docker compose -f cluster-a/compose.yaml up -d
docker compose -f cluster-b/compose.yaml up -d
docker compose -f cluster-c/compose.yaml up -d
```

Check that each node started its REST API. A node needs a few seconds to start, so if the command prints nothing, wait and run it again:

```sh
docker logs patrolli-a-1 2>&1 | grep "starting API server"
```

The output looks like this:

```text
2026-10-22T09:01:31.118486960+00:00  INFO patrolli: starting API server addr=0.0.0.0:8080 tls=false
```

Run the same command for `patrolli-b-1` and `patrolli-c-1`.

## 3. Start the web interface

The web interface reads the list of clusters from `config-fe.yaml`. Create the Compose file and the configuration file:

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
    - name: Cluster A
      url: http://patrolli-a-1:8080
    - name: Cluster B
      url: http://patrolli-b-1:8080
    - name: Cluster C
      url: http://patrolli-c-1:8080
```

:::

Each `url` is the REST API of a cluster. The `patrolli-web` container reaches the nodes by container name because it is on the same Docker network. If a cluster runs on another host, use an address that the `patrolli-web` container can reach.

Start the web interface:

```sh
docker compose -f web/compose.yaml up -d
```

## 4. Sign in

1. **Open the web interface**

    Open `http://192.168.1.10:3000` in a browser. The **Cluster** list shows the clusters from `config-fe.yaml`.

    ![The sign-in page lists Cluster A, Cluster B, and Cluster C](./assets/multiple-cluster/sign-in-clusters.png)

2. **Sign in to a cluster**

    Pick **Cluster A**. Enter the username `admin` and the password `change-me`, and click **Sign in**.

3. **Check the cluster**

    Click **Cluster Monitoring**. The page lists the nodes of the cluster that you signed in to. The box at the top right shows the name of that cluster.

    ![Cluster Monitoring shows node-1 of Cluster A as active and leader](./assets/multiple-cluster/cluster-monitoring.png)

A username and a password sign you in to one cluster only, because each cluster has its own users. To work on another cluster, open the user menu at the top right, click **Log Out**, and sign in again with the other cluster.

The **Sign in with Keycloak** button works after you set up single sign-on. Single sign-on also lets you switch clusters without signing in again. See [Authentication](./authentication).

## Troubleshooting

1. **Failed to communicate with cluster**

    ![The Bad Gateway message says: Failed to communicate with cluster Cluster C](./assets/multiple-cluster/bad-gateway.png)

    The web interface cannot reach the REST API of the cluster. To find the cause:

    - **Check the node:** run `docker ps --filter name=patrolli-c-1`. If the node is not running, read its log with `docker logs patrolli-c-1`.
    - **Check the web interface log:** run `docker logs patrolli-web 2>&1 | grep "Cluster C"`. A line such as `getaddrinfo ENOTFOUND patrolli-c-1` means that the name in `url` does not resolve from the `patrolli-web` container.
    - **Check the address:** the `url` in `web/config-fe.yaml` must point to a node that has the `api` block in its `config.yaml`.

## Remove the deployment

To delete the containers, the network, and all data of this guide, run:

```sh
docker compose -f web/compose.yaml down
docker compose -f cluster-a/compose.yaml down -v
docker compose -f cluster-b/compose.yaml down -v
docker compose -f cluster-c/compose.yaml down -v
docker compose -f postgres/compose.yaml down -v
docker network rm patrolli
```

## Next steps

- Sign in to all clusters with one account in [Authentication](./authentication).
- Learn how the nodes of one cluster share the work in [Clustering](./clustering).
