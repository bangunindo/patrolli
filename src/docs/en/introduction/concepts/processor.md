---
description: A processor changes data on its way from items to sinks. Processors connect to each other as a graph.
---

# Processor

A processor changes data on its way from [items](./item) to [sinks](./sink). It can drop records that you do not need, add or rename attributes, or look up extra information in a table, such as the country of an IP address.

## Processor types

Patrolli has eight processor types.

- `null` passes data through unchanged. Use it to join several streams into one, or to split one stream into several paths.
- `attribute_filter` keeps the records that match a condition and drops the rest. A condition compares an attribute with a value, and you can combine conditions with `and` and `or`.
- `add_attributes` sets attributes to fixed values, such as `site: hq`.
- `rename_attributes` renames attributes, such as `host.name` to `device.name`.
- `drop_attributes` removes attributes that you do not need.
- `attribute_lookup` looks up values in a lookup table and writes them as attributes. It can add GeoIP data to flow records, or interface names to the interface numbers in NetFlow records.
- `attribute_collect` writes values from incoming records into a lookup table. It can save the interface names from an SNMP walk, for example, so that an `attribute_lookup` processor can add those names to NetFlow records.
- `transform` runs a short program in OPL, the OpenTelemetry Processing Language. Use it when the other types cannot do what you need. This type is experimental.

Each attribute processor works on one of three attribute tables. The `record` table holds the attributes of each log, span, or metric. The `scope` table holds the attributes of the instrumentation scope. The `resource` table holds the attributes of the resource, including the host and item details that Patrolli adds.

## How processors connect

Processors do not form a fixed chain. They form a graph that you build from three kinds of links. An item lists the processors that get its data in its `processors` field. A processor lists the processors it reads from in its `inputs` field, and so does a sink.

The output of a processor goes to every processor and every sink that lists it in `inputs`, and each of them gets its own copy. A processor that lists several inputs merges those streams into one.

The graph cannot have a loop. A processor cannot read from its own output, directly or through other processors.

If no processor and no sink reads from a processor, Patrolli drops the output of that processor. Patrolli logs a warning when this happens, because it is usually a mistake.

## Example

Suppose you receive NetFlow from your routers. You want only the flows that go through one next hop, and you want the country of each source address. You connect the objects like this:

```
Item (netflow) --> only-next-hop --> add-geoip --> Sink (ClickHouse)
```

In the YAML file, the two processors look like this:

```yaml
processors:
  - id: 019c89fd-632a-70a2-b02b-c43fa9c96a3d
    name: only-next-hop
    type: attribute_filter
    config:
      match:
        table: record
        key: nf.data.ipNextHopIPv4Address
        key_is_regex: false
        op: "="
        value: {string: 10.0.0.1}

  - id: 01a05abf-25ff-70a2-89df-0eb8f1ab10aa
    name: add-geoip
    type: attribute_lookup
    inputs:
      - 019c89fd-632a-70a2-b02b-c43fa9c96a3d
    config:
      lookups:
        - lookup_table: 01a08fdb-8761-70a2-999a-cdc856cfc5fa
          table: record
          match:
            point:
              key: nf.data.sourceIPv4Address
              cast: ip
          output:
            "geoip.source.country_code": country_code
```

The netflow item lists `only-next-hop` in its `processors` field, and the ClickHouse sink lists `add-geoip` in its `inputs` field.

## Disabled processors

With a backend, you can disable a processor. A disabled processor stays in the graph and passes data through unchanged, like a `null` processor. The processors and sinks after it still receive data.
