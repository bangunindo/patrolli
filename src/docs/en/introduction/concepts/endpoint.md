---
description: An endpoint is an address on a host that Patrolli can connect to, together with its credentials.
---

# Endpoint

An endpoint tells Patrolli how to reach a [host](./host). It holds an address and the credentials for that address, such as an SNMP community string or an HTTP token.

Poll [items](./item) use endpoints. An `snmp` item needs an SNMP endpoint to know which device to ask and which credentials to send. A `prometheus_scrape` item needs an HTTP endpoint to know which URL to read.

Each endpoint belongs to one host. A host can have several endpoints, for example one for SNMP and one for an HTTP exporter on the same machine.

## Endpoint types

Patrolli has four endpoint types.

- `snmp_v1` is an SNMP v1 agent. It needs an `address` and a `community`.
- `snmp_v2c` is an SNMP v2c agent. It needs an `address` and a `community`. Walks use GETBULK by default, with 25 rows per request. Set `max_bulk_repetitions` to change the number of rows, or set `disable_bulk: true` for a device that handles GETBULK badly.
- `snmp_v3` is an SNMP v3 agent. Instead of a community, it has a `security_model` with the user name, the authentication protocol, and the privacy protocol. It has the same bulk options as `snmp_v2c`.
- `http` is an HTTP or HTTPS URL. It can have basic or bearer authentication, TLS settings, and a `connect_timeout`. `prometheus_scrape` items use this type.

If an SNMP address has no port, Patrolli uses port 161.

This host has an SNMP endpoint and an HTTP endpoint:

```yaml
endpoints:
  - id: 019c65dc-3e74-70a2-875c-6035bf8990f6
    name: router-office-snmp
    host_id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    type: snmp_v2c
    config:
      address: 192.168.1.1:161
      community: public

  - id: 019c65dc-3e74-70a2-875c-6035bf8990f9
    name: router-office-exporter
    host_id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    type: http
    config:
      url: https://192.168.1.1:9100/metrics
      auth:
        Bearer: "exporter-token"
```

## Why endpoints are separate from items

A device often has dozens of items that all talk to the same agent. Because the address and the credentials are on the endpoint, you write them once. When the community string changes, you update one endpoint, and every item that uses it gets the new value.

Patrolli also uses the endpoint to save requests. When several `snmp` items use the same endpoint, Patrolli asks for all their OIDs in one SNMP GET request.

## Endpoints and listeners

A listener item can also have an endpoint. When you set its `endpoint_id`, the endpoint has two uses for the item. It gives the item an IP to match records against, and it labels the records that the item takes.

The router in the example above shows both uses. Patrolli polls the router through the `router-office-snmp` endpoint, with the address `192.168.1.1:161`. The router also sends syslog to a `syslog` listener on Patrolli. That listener receives syslog from every device in the network, so Patrolli needs a way to tell which messages come from the router.

This listener item uses the same endpoint:

```yaml
items:
  - id: 019bd3f0-f429-70a2-8602-508e4a18f8b9
    name: router-office-syslog
    type: listener
    host_id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    listener_id: 019b77b3-df79-70a2-ab01-b43b2cf723a6
    endpoint_id: 019c65dc-3e74-70a2-875c-6035bf8990f6
    config:
      match_strategy:
        method: endpoint_source_ip
```

With the `endpoint_source_ip` match strategy, Patrolli takes the IP from the endpoint address, `192.168.1.1`, and ignores the port. Every syslog message sent from `192.168.1.1` goes to this item, and so to the host of the router. You write the IP of the router once, on the endpoint. If the IP changes, you update the endpoint, and both the polling and the syslog matching use the new IP.

This match needs an IP address on the endpoint. If the endpoint address is a hostname such as `router.office.local:161`, Patrolli skips the item, and the messages from the router go to the next item that matches. The other match strategies do not use the endpoint IP. The [item](./item) page explains all of them.

The second use applies to every match strategy. Patrolli adds the endpoint to each record that the item takes, as the resource attributes `patrolli.endpoint.id`, `patrolli.endpoint.name`, and `patrolli.endpoint.type`. In the example, each syslog message from the router has `patrolli.endpoint.name` set to `router-office-snmp`, the same value as the records from the SNMP polls.
