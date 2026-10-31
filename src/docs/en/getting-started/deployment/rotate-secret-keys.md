---
description: Replace the key that encrypts the stored secrets of a Patrolli deployment with a backend, without losing the secrets.
---

# Rotate Secret Keys

A node with a backend encrypts the passwords, passphrases, and tokens that it keeps in the database. It encrypts new values with the key that `PATROLLI_SECRET_ACTIVE_KEY` names, and it can decrypt values with every key in `PATROLLI_SECRET_KEYS`. To replace the active key, you add a new key, make the new key active, and remove the old key. The stored secrets stay readable during every step.

These steps work for any deployment with a backend. You need a way to change the environment variables of each node and to restart each node. The examples show a Docker Compose service named `patrolli`. If you deploy in another way, set the same values in your own setup.

## Before you begin

- A deployment with a backend. This page calls its current key `k1` and the new key `k2`.
- A secret for the new key, at least 16 bytes long and without commas. A long random value is safest.
- In a cluster, access to every node. Finish each step on every node before you start the next step.

## 1. Add the new key

Create the new key once. `openssl` is one way to make a long random value:

```sh
openssl rand -base64 32
```

Add the new key to `PATROLLI_SECRET_KEYS` with the id `k2`, after the old key. Keep `k1` as the active key. With Docker Compose, the change in the `environment` of the node looks like this:

::: code-group

```yaml [compose.yaml]
services:
    patrolli:
        environment:
            PATROLLI_SECRET_KEYS: k1:<old key> # [!code --]
            PATROLLI_SECRET_KEYS: k1:<old key>,k2:<new key> # [!code ++]
            PATROLLI_SECRET_ACTIVE_KEY: k1
```

:::

Restart the node. With Docker Compose, `docker compose up -d` creates the container again when its environment changed.

::: warning
In a cluster with multiple nodes, add the same `k2` value to every node. A node without `k2` cannot read the secrets that the other nodes encrypt with `k2` in the next step.
:::

Use a new id for every new key. Never give an old id to a different key.

## 2. Make the new key active

Set `PATROLLI_SECRET_ACTIVE_KEY` to `k2`:

::: code-group

```yaml [compose.yaml]
services:
    patrolli:
        environment:
            PATROLLI_SECRET_KEYS: k1:<old key>,k2:<new key>
            PATROLLI_SECRET_ACTIVE_KEY: k1 # [!code --]
            PATROLLI_SECRET_ACTIVE_KEY: k2 # [!code ++]
```

:::

Restart the node. At startup, the node encrypts every stored secret again with `k2` and logs the result. With Docker Compose, find the line with:

```sh
docker compose logs patrolli | grep "secret values re-encrypted"
```

The line looks like this:

```text
2026-10-29T07:38:07.537575369+00:00  INFO patrolli::db: secret values re-encrypted reencrypted=12 remaining=0 active_key_id="k2"
```

- **`reencrypted`:** the number of secrets that this node encrypted again with `k2`.
- **`remaining`:** the number of database rows that still have a secret on another key. When `remaining` is above `0`, the line is a `WARN` line.

In a cluster, restart every node. The first node that restarts does most of the work. A node that does not restart yet still encrypts the secrets that its REST API receives with `k1`, so a later node can show `reencrypted` above `0`. Read `remaining` in the log of the last node that you restart.

During this step, saving a form that was open in the web interface before the restart fails with `no stored value has this {"set": ...} handle`. Reload the page and save again.

## 3. Remove the old key

When `remaining` is `0`, remove `k1` from `PATROLLI_SECRET_KEYS`:

::: code-group

```yaml [compose.yaml]
services:
    patrolli:
        environment:
            PATROLLI_SECRET_KEYS: k1:<old key>,k2:<new key> # [!code --]
            PATROLLI_SECRET_KEYS: k2:<new key> # [!code ++]
            PATROLLI_SECRET_ACTIVE_KEY: k2
```

:::

Restart the node. In a cluster, do this on every node.

::: warning
Do not remove `k1` while `remaining` is above `0`. A node that cannot decrypt a secret that it needs does not load its configuration, and its log repeats `failed to load the configuration, retrying`.
:::

## Troubleshooting

1. **`remaining` is above `0`**

    The node could not encrypt some secrets again. It logs a warning for each of them:

    ```sh
    docker compose logs patrolli | grep "secret value could not be re-encrypted"
    ```

    Each warning names the `table` (`sinks`, `listeners`, or `host_endpoints`) and the `row_id` of the item. Open that sink, listener, or endpoint in the web interface, type each of its secrets again, and save. Saving without typing the secrets keeps the values that failed.

    Then restart the node, and check that the last `secret values re-encrypted` line shows `remaining=0`. With Docker Compose, the environment did not change, so `docker compose up -d` does not restart the container. Run `docker compose restart patrolli` instead.
