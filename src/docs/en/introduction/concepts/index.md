---
description: The building blocks of Patrolli and how they connect to each other.
---

# Concepts

Patrolli has six kinds of objects. You connect them to describe where data comes from, what happens to it, and where it goes. The YAML file and the web interface are both organized around these six objects.

## The building blocks

- A [host](./host) is a device or service that you monitor, such as a router, a firewall, or a server. Endpoints and items always belong to a host.
- An [endpoint](./endpoint) is an address on a host that Patrolli can connect to, together with the credentials it needs. An SNMP agent with its community string is an endpoint, and so is an HTTP URL.
- A [listener](./listener) is a port that Patrolli opens to receive data that devices send on their own, such as syslog, NetFlow, or SNMP traps.
- An [item](./item) is one thing to collect from a host. It either polls an endpoint on a schedule or selects the records for its host from a listener.
- A [processor](./processor) changes data on its way through. It can filter records, add or rename attributes, or look up extra information in a table.
- A [sink](./sink) is a destination for the data, such as a file, Kafka, ClickHouse, or an OpenTelemetry collector.

## How the objects connect

Data always starts at an item, goes through zero or more processors, and ends at one or more sinks.

```
Endpoint --> Item (poll) ----------+
                                   |
                                   v
                               Processor --> Processor --> Sink
                                   ^                   |
                                   |                   +--> Sink
Listener --> Item (listener) ------+
```

## Example

Suppose you want to watch an office router. You want its interface table every minute over SNMP, and you also want the syslog messages it sends. You set up Patrolli in five steps.

1. **Create a host.** Add a host named `router-office`. Patrolli writes this name on every record that comes from the router.

2. **Add an endpoint.** Add an `snmp_v2c` endpoint to the host with the address of the router, for example `192.168.1.1:161`, and its community string.

3. **Add a poll item.** Add an `snmp_walk` item to the host. It uses the endpoint from the previous step, walks the interface OIDs, and runs every 60 seconds.

4. **Add a listener item.** Make sure that a `syslog` listener exists. Then add a `listener` item to the host that points at that listener and uses the `endpoint_source_ip` match strategy. Syslog messages with the source address `192.168.1.1` now belong to `router-office`.

5. **Route the data.** Connect both items to a processor, for example one that adds a `site` attribute. Then connect that processor to a sink that writes to ClickHouse.

## Where you define the objects

The objects are the same in every way of running Patrolli. Only the place where you define them changes.

On a single node without a backend, you list every object in the YAML file under `hosts`, `endpoints`, `listeners`, `items`, `processors`, and `sinks`. Each object has an `id`, which is a UUID, and objects refer to each other by that `id`. Patrolli reads the file at startup, so a change needs a restart.

With a backend, Patrolli stores the objects in PostgreSQL. You manage them through the REST API or the web interface, and changes apply without a restart.
