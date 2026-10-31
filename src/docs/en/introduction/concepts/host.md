---
description: A host is a device or service that you monitor. Endpoints and items belong to a host.
---

# Host

A host is a device or a service that you monitor, such as a router, a switch, a firewall, a server, or an application.

Every [endpoint](./endpoint) and every [item](./item) belongs to exactly one host. A host can have many of both. A server, for example, can have one endpoint for SNMP and another for a Prometheus exporter, with several items on each.

A host has no address of its own. Addresses live on its endpoints, because you can often reach one device in more than one way.

## What a host holds

Besides its `id`, a host has a name and a time zone. Patrolli writes the name, such as `router-office`, on every record that comes from the host. The time zone is optional. It is the zone that the device uses for its own timestamps, written as an IANA name such as `Asia/Jakarta`. Leave it empty for UTC.

In the YAML file, a host looks like this:

```yaml
hosts:
  - id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    name: router-office
    timezone: Asia/Jakarta
```

## The host in your data

When a record passes through an item, Patrolli adds the details of the host to the record as resource attributes.

| Attribute | Value |
|---|---|
| `host.name` | The host name |
| `host.id` | The host ID |
| `service.name` | The host name |
| `service.instance.id` | The host ID |
| `patrolli.host.timezone` | The host time zone, if it is set |

These are standard OpenTelemetry attribute names, so most tools can group and filter by host with no extra setup. The [item](./item) page has the full list of attributes that Patrolli adds.

## Hosts in a cluster

In a cluster, the leader node assigns each host to one node, and that node does all the work for the host. Only the owner node runs the poll items of the host. Every node runs the listeners, and a node that receives records for a host it does not own forwards them to the owner node.

When a node goes down, the leader moves its hosts to the nodes that still run. The [clustering](../../getting-started/deployment/clustering) guide shows this step by step.

## Templates

With a backend, you can define items once in a template instead of on every host. When you apply a template to a host, Patrolli adds the items of the template to that host. Use templates when many devices of the same model need the same items.
