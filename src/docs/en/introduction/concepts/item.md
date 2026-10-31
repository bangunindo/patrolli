---
description: An item is one thing to collect from a host, either by polling an endpoint or by taking records from a listener.
---

# Item

An item is one thing that you want to collect from a [host](./host). The interface table of a router is an item. So are the uptime of a switch and the syslog messages from a firewall.

Every item belongs to one host and lists the [processors](./processor) that get its data. A poll item asks a device for data on a schedule, through an [endpoint](./endpoint). A listener item waits for a device to send data to a [listener](./listener), then selects the records that belong to its host.

## Poll items

A poll item runs a check on a schedule. Patrolli has four poll item types.

- `snmp` gets the value of one OID, such as `sysUpTime.0`. It needs an SNMP endpoint. When several `snmp` items use the same endpoint, Patrolli asks for all their OIDs in one request.
- `snmp_walk` walks one or more OIDs, such as the columns of the interface table. It needs an SNMP endpoint. With `table_format: true`, Patrolli joins the columns by index, and each row of the table becomes one record.
- `prometheus_scrape` reads a Prometheus metrics page. It needs an `http` endpoint.
- `dummy` produces a fixed log record for testing. It needs no endpoint.

The `schedule` field sets when a check runs. It takes a number of seconds such as `"60"`, a cron expression that starts with a seconds field such as `"0 */5 * * * *"`, or `"never"`. The default is 300 seconds.

The `timeout` field limits how long the whole check can take, in milliseconds. The default is 1000. A walk of a large table often needs more.

Patrolli spreads the items that share an interval across that interval, so a thousand items with a 60-second schedule do not all start in the same second. If a check is still running when its next turn comes, Patrolli skips that turn. It never runs two checks of the same item at once.

This item walks the interface table of a router every 60 seconds:

```yaml
items:
  - id: 019c65dc-6f10-70a2-ad19-b7c6eadeff91
    name: interfaces-table
    host_id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    endpoint_id: 019c65dc-3e74-70a2-875c-6035bf8990f6
    type: snmp_walk
    schedule: "60"
    timeout: 30000
    processors:
      - 019c98c8-5512-70a2-906e-7ccd03618702
    config:
      table_format: true
      oids:
        - oid: ifIndex
        - oid: ifDescr
        - oid: ifName
        - oid: ifOperStatus
```

## Listener items

A listener item has the type `listener` and points at a listener with `listener_id`. Its `match_strategy` decides which incoming records belong to it. There are four strategies.

- `endpoint_source_ip` takes the records whose source IP is the IP of the endpoint of the item. The endpoint address must be an IP address, not a hostname.
- `source_ips` takes the records whose source IP is in a list that you give.
- `attribute_match` takes the records that match an attribute filter, written with the same syntax as the `attribute_filter` processor. Use it when the source IP does not identify the device, for example when the messages come through a relay.
- `none` takes all records. It is the catch-all.

When a record arrives, Patrolli tries the items of the listener in a fixed order:

1. `endpoint_source_ip` items
2. `source_ips` items
3. `attribute_match` items
4. `none` items

The record goes to the first item that matches, and to no other item. So you can give each device its own item, and add one `none` item for everything else.

This item takes the syslog messages that come from the address of the router endpoint:

```yaml
items:
  - id: 019bd3f0-f429-70a2-8602-508e4a18f8b9
    name: router-syslog
    host_id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    endpoint_id: 019c65dc-3e74-70a2-875c-6035bf8990f6
    listener_id: 019b77b3-df79-70a2-ab01-b43b2cf723a6
    type: listener
    processors:
      - 019c98c8-5512-70a2-906e-7ccd03618702
    config:
      match_strategy:
        method: endpoint_source_ip
```

A listener item has no schedule. It handles records as they arrive.

## Where the data of an item goes

The `processors` field lists the processors that receive the data of the item. If the field lists more than one processor, each one gets its own copy.

An item with an empty `processors` list still sends its data. The data goes to the sinks that have no inputs, which the [sink](./sink) page explains.

## What Patrolli adds to each record

Patrolli adds the details of the item, its host, and its endpoint to each record as resource attributes. Later in the pipeline, these attributes tell you where a record came from.

| Attribute | Value |
|---|---|
| `host.name`, `service.name` | The host name |
| `host.id`, `service.instance.id` | The host ID |
| `patrolli.host.timezone` | The host time zone, if it is set |
| `patrolli.item.id` | The item ID |
| `patrolli.item.name` | The item name |
| `patrolli.endpoint.id` | The endpoint ID, if the item has an endpoint |
| `patrolli.endpoint.name` | The endpoint name, if the item has an endpoint |
| `patrolli.endpoint.type` | The endpoint type, such as `snmp_v2c`, if the item has an endpoint |

Listener records that match no item do not get these attributes.

## Item status

With a backend, Patrolli keeps a status for each item, and the web interface shows it.

- `never_ran` means the item has not run a check or received a record yet.
- `ok` means the last check worked and returned data.
- `ok_empty` means the last check worked but returned no data.
- `retrying` means the last check failed and Patrolli will try again.
- `failing` means the check failed and the retries failed too.
- `stale` means the check is late by more than its schedule allows. This status usually points to a problem in Patrolli or in the node, not in the device.
