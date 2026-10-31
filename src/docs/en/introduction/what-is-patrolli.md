---
description: Patrolli collects telemetry from legacy and modern sources, processes it, and delivers it to the systems you already use.
---

# What is Patrolli?

Patrolli is a monitoring and telemetry platform. It collects data from your network devices and services, processes it, and sends it to storage and analytics systems.

Many networks still depend on older protocols such as SNMP, syslog, and NetFlow. Patrolli receives this data and converts it to OpenTelemetry. You can then use modern tools with it, and you do not need to replace your devices.

## How it works

Data moves through Patrolli in three stages:

1. **Collect**

    Patrolli gets data in two ways:
    - **Listen:** Patrolli receives data that devices send to it: syslog, NetFlow and sFlow, SNMP traps, and OpenTelemetry.
    - **Poll:** Patrolli requests data from devices on a schedule: SNMP get, SNMP walk, and Prometheus scrape.

2. **Process**

    **Processors** change the data before it is delivered. They can filter, add, drop, or rename attributes, enrich records from lookup tables, or run a transform program.

3. **Deliver**

    **Sinks** send the result to its destination: a file, Kafka, ClickHouse, or an OpenTelemetry endpoint.

Each item belongs to a **host** and sends its data to one or more processors. Processors and sinks connect as a graph, so one stream of data can go to several destinations.

Inside Patrolli, data stays in the [OpenTelemetry Arrow](https://github.com/open-telemetry/otel-arrow) format from start to end for achieving high performance.

## Ways to run Patrolli

- **Single node without a backend:** All configuration comes from one YAML file. This is the simplest setup.
- **Single node with a backend:** Configuration is stored in PostgreSQL. You manage it through the REST API or the web interface, and changes apply without a restart.
- **Cluster:** Several nodes share one PostgreSQL database. One node is the leader. It spreads hosts across the nodes and moves them to a spare node when a node goes down.
- **Multiple clusters:** Several clusters run side by side, and each cluster has its own PostgreSQL database. You manage all of them from one web interface and switch between clusters there.

## Next steps

- Read about the [concepts](./concepts/) that Patrolli uses.
- Follow the [setup guide](../getting-started/deployment/) to install Patrolli.
