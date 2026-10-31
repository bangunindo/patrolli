---
description: A listener opens a port and receives data that devices send to Patrolli, such as syslog, NetFlow, and SNMP traps.
---

# Listener

A listener opens a port and waits for data. The device decides when to send, and Patrolli receives what arrives.

## Listener types

Patrolli has seven listener types.

- `syslog` receives syslog over UDP, TCP, and TLS, and one listener can use all three at the same time. Besides the RFC 3164 and RFC 5424 formats, it recognizes CEF, Cisco ISE and ACS, and the Cisco IOS, IOS XR, NX-OS, ASA, and FTD formats. It turns their fields into attributes that you can filter on later.
- `netflow` receives NetFlow v5, NetFlow v9, and IPFIX over UDP. The same port also accepts sFlow, so sFlow does not need its own listener.
- `snmp_trap` receives SNMP v1, v2c, and v3 traps over UDP.
- `open_telemetry` receives OTLP, OTAP, or both over gRPC. Use it for OpenTelemetry SDKs and collectors, or for the `open_telemetry` sink of another Patrolli node.
- `internal_logs` turns the log messages of Patrolli itself into log records. It opens no port.
- `internal_metrics` sends the internal counters of Patrolli, such as throughput and queue sizes, at a fixed interval. It opens no port.
- `dummy` produces fake records at a fixed rate, for testing a pipeline.

## A listener does not belong to a host

One syslog listener on port 514 can receive messages from hundreds of devices, so a listener is not tied to a host.

A listener [item](./item) connects a listener to a host. Each listener item belongs to one host and decides which incoming records are for that host, for example every record with the source IP `192.168.1.1`. When a record arrives, Patrolli finds its item and adds the host and item details to the record. Then Patrolli sends the record to the processors of that item.

Patrolli does not drop a record that matches no item. The record continues without host details and goes to the sinks that have no inputs. The [sink](./sink) page explains these catch-all sinks.

## A minimal listener

This listener receives syslog on UDP port 514:

```yaml
listeners:
    - id: 019b77b3-df79-70a2-ab01-b43b2cf723a6
      name: syslog-1
      type: syslog
      config:
          listener:
              udp:
                  address: 0.0.0.0:514
```

Every listener can also set `worker_count`, `batch_size`, and `batch_timeout` next to `type`. The `worker_count` field sets the number of workers that parse and match the incoming records. Patrolli groups records into batches before it sends them on, and a batch goes out when it reaches `batch_size` records or after `batch_timeout` milliseconds. Both default to 1000.

The fields under `config` depend on the listener type. The UDP listeners have a second `worker_count` inside `config.listener`. That one sets the number of sockets that read from the port, and it is separate from the outer `worker_count`.

## Listeners in a cluster

In a cluster, every node runs every listener, except standby nodes. You can point a device at any node, or put a load balancer in front of the nodes.

Each host has one owner node. When a node receives a record for a host that another node owns, it forwards the record to the owner. This way, one node processes all the data of a host. To process records on the node that received them instead, set `disable_cluster_forwarding: true` on the listener.
