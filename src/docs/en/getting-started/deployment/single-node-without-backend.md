---
description: Run one Patrolli node that reads its whole pipeline from a YAML file, with no database and no web interface.
---

# Single Node without Backend

This guide runs one Patrolli node with Docker Compose. The node reads its whole configuration from one YAML file: what to collect, what to do with the data, and where to send it. It needs no database, and it has no REST API and no web interface.

The example pipeline receives syslog on UDP port `514` and writes every message to a file.

## Before you begin

You need the following:

- A Linux host with Docker Engine and the Docker Compose plugin.
- The `patrolli-core` image on that host.
- `logger` on the host, to send a test message. It is part of util-linux.

## 1. Write the configuration

Create a directory for the deployment:

```sh
mkdir patrolli-local && cd patrolli-local
```

Create the configuration file:

::: code-group

```yaml [config.yaml]
global:
    logger:
        level: info

listeners:
    - type: syslog
      id: 01a16349-af0a-708e-9198-474373276c0b
      name: syslog
      config:
          listener:
              udp:
                  address: 0.0.0.0:514

hosts:
    - id: 01a16349-af0d-7834-a9de-9015324d9a46
      name: router-1

processors:
    - id: 01a16349-af10-7ee0-8312-f289804229c8
      name: passthrough
      type: "null"

items:
    - type: listener
      id: 01a16349-af13-7d6e-ba18-3e3cc7b8d449
      name: router-1-syslog
      host_id: 01a16349-af0d-7834-a9de-9015324d9a46
      listener_id: 01a16349-af0a-708e-9198-474373276c0b
      processors:
          - 01a16349-af10-7ee0-8312-f289804229c8
      config:
          match_strategy:
              method: none

sinks:
    - type: file
      id: 01a16349-af16-785e-a3c4-2445e03bf030
      name: syslog-file
      inputs:
          - 01a16349-af10-7ee0-8312-f289804229c8
      config:
          path: /var/log/patrolli/syslog.log
          format: json_nested_denormalized
```

:::

The file has one entry in each section:

- **`global`:** settings for the whole node. This example sets only the log level, but the node does not start without the block.
- **`listeners`:** where data comes in. The `syslog` listener receives syslog over UDP on port `514`.
- **`hosts`:** the devices that you monitor. `router-1` stands for the device that sends the syslog.
- **`processors`:** what happens to the data on its way. The `null` processor passes every record through unchanged.
- **`items`:** the links between a listener, a host, and processors. With `match_strategy` set to `none`, the item takes every message that the listener receives.
- **`sinks`:** where the data goes. The `file` sink writes one JSON record per line to `/var/log/patrolli/syslog.log`.

Every entry has an `id`, a UUID that other entries use to point to it. The item names its listener in `listener_id`, its host in `host_id`, and its processor in `processors`. The sink takes its records from the processor in `inputs`. To read more about each part, see [Concepts](../../introduction/concepts/).

## 2. Start the node

Create the Compose file:

::: code-group

```yaml [compose.yaml]
name: patrolli-local

services:
    patrolli:
        image: patrolli-core
        container_name: patrolli
        restart: unless-stopped
        ports:
            - "514:514/udp"
        volumes:
            - ./config.yaml:/config.yaml:ro
            - data:/var/lib/patrolli
            - logs:/var/log/patrolli

volumes:
    data:
    logs:
```

:::

The `data` volume keeps the directory where the node stores queues on disk. The `logs` volume holds the file that the sink writes.

Start the node:

```sh
docker compose up -d
```

Check that the node started. If the command prints nothing, wait a few seconds and run it again:

```sh
docker logs patrolli 2>&1 | grep "startup complete"
```

The output looks like this:

```text
2026-10-22T11:05:59.154717852+00:00  INFO patrolli: startup complete listeners=1 sinks=1 items=1
```

## 3. Send a test message

Send a syslog message to the node:

```sh
logger --udp --server 127.0.0.1 --port 514 "hello from the Patrolli guide"
```

Find the message in the file. The sink can take a few seconds to write it, so if the command prints nothing, run it again:

```sh
docker exec patrolli grep "hello from the Patrolli guide" /var/log/patrolli/syslog.log
```

The output is one JSON record. Formatted, and with some fields left out, it looks like this:

```json
{
    "kind": "logs",
    "resource": {
        "attributes": {
            "host": {
                "id": "01a16349-af0d-7834-a9de-9015324d9a46",
                "name": "router-1"
            },
            "patrolli": {
                "item": {
                    "name": "router-1-syslog",
                    "id": "01a16349-af13-7d6e-ba18-3e3cc7b8d449"
                }
            }
        }
    },
    "scope": {
        "name": "patrolli.syslog",
        "attributes": {
            "patrolli": {
                "listener": {
                    "id": "01a16349-af0a-708e-9198-474373276c0b",
                    "type": "syslog",
                    "name": "syslog"
                }
            }
        }
    },
    "record": {
        "attributes": {
            "protocol": "RFC5424",
            "severity_level": "notice"
        },
        "body": "hello from the Patrolli guide"
    }
}
```

Patrolli adds the host, the item, and the listener to every record, so later processors and sinks can tell the records apart.

To collect syslog from a real device, set the device to send syslog to the IP address of the host, on UDP port `514`.

## 4. Change the configuration

The node reads `config.yaml` only when it starts. After you change the file, restart the node:

```sh
docker compose restart
```

## Remove the deployment

To delete the container and its volumes, run:

```sh
docker compose down -v
```

## Next steps

- Keep the configuration in a database and change it in the web interface, without a restart, in [Single Node with Backend](./single-node-with-backend).
- Read about listeners, items, processors, and sinks in [Concepts](../../introduction/concepts/).
