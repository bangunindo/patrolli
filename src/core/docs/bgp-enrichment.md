<!--
SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya

SPDX-License-Identifier: AGPL-3.0-only
-->

# Enriching flows with BGP routes

A `bgp` listener keeps each router's routes in a `bgp` lookup table, fed over BMP
(Loc-RIB) or a passive BGP session. An `attribute_lookup` rule then adds the origin
AS, AS path, communities, prefix and next hop of each flow's destination (or source),
as that flow's router saw them.

This guide is the how-to. The design and its decisions are in
[`bgp-bmp.md`](bgp-bmp.md); every option is annotated in `config-full.yaml`.

## 1. The pieces

| Piece | What it does |
|---|---|
| `bgp` listener | Accepts BMP (`bmp:`) and/or BGP (`bgp:`) sessions from routers, and writes their routes into the table. |
| `bgp` lookup table | Holds the routes, partitioned by `router` (the session's source address) and `vrf` (`global`, or the RD). |
| `netflow` listener | Receives the flows: NetFlow v5/v9, IPFIX, sFlow. |
| `attribute_lookup` | Finds the longest prefix holding a flow's address in its router's partition. |

A complete local config (the YAML below is checked by
`config::tests::the_bgp_enrichment_guide_validates`, so it stays current):

<!-- BEGIN validated config -->
```yaml
lookup_tables:
  - id: 019c98c8-5512-70a2-906e-7ccd03618800
    name: routes
    source:
      type: bgp
      listener: 019c98c8-5512-70a2-906e-7ccd03618801
  # exporter → router, for exporters that are not the BGP/BMP speaker (§3)
  - id: 019c98c8-5512-70a2-906e-7ccd03618802
    name: exporters
    source:
      type: file
      location:
        type: file_system
      paths: [/etc/patrolli/exporters.csv]   # exporter,router
      reload_interval: 60
      format:
        type: csv
        has_headers: true
    key:
      type: hash
      key_columns: [exporter]

listeners:
  - id: 019c98c8-5512-70a2-906e-7ccd03618801
    name: routes
    type: bgp
    config:
      bmp:
        listener:
          address: 0.0.0.0:11019
      bgp:
        listener:
          address: 0.0.0.0:179   # needs CAP_NET_BIND_SERVICE
  - id: 019c98c8-5512-70a2-906e-7ccd03618803
    name: flows
    type: netflow
    config:
      listener:
        address: 0.0.0.0:2055

hosts:
  - id: 019c98c8-5512-70a2-906e-7ccd03618804
    name: edge

processors:
  - id: 019c98c8-5512-70a2-906e-7ccd03618805
    name: map-exporters
    type: attribute_lookup
    config:
      lookups:
        - lookup_table: 019c98c8-5512-70a2-906e-7ccd03618802
          table: record
          match:
            keys:
              exporter: {key: source.address}
          output:
            bgp.router: router
  - id: 019c98c8-5512-70a2-906e-7ccd03618806
    name: enrich
    type: attribute_lookup
    inputs: [019c98c8-5512-70a2-906e-7ccd03618805]
    config:
      lookups:
        - lookup_table: 019c98c8-5512-70a2-906e-7ccd03618800
          table: record
          match:
            keys:
              # the mapped router where there is one, else the exporter
              router: {key: [bgp.router, source.address], cast: ip}
            literals:
              vrf: global
            point:
              key: [nf.data.destinationIPv4Address, nf.data.destinationIPv6Address]
          output:
            dst.as: origin_as
            dst.as_path: as_path
            dst.communities: communities
            dst.network: prefix
            dst.next_hop: next_hop
        - lookup_table: 019c98c8-5512-70a2-906e-7ccd03618800
          table: record
          match:
            keys:
              router: {key: [bgp.router, source.address], cast: ip}
            literals:
              vrf: global
            point:
              key: [nf.data.sourceIPv4Address, nf.data.sourceIPv6Address]
          output:
            src.as: origin_as
            src.network: prefix

items:
  - type: listener
    id: 019c98c8-5512-70a2-906e-7ccd03618807
    name: flows
    host_id: 019c98c8-5512-70a2-906e-7ccd03618804
    listener_id: 019c98c8-5512-70a2-906e-7ccd03618803
    processors: [019c98c8-5512-70a2-906e-7ccd03618805]
    config:
      match_strategy:
        method: none

sinks:
  - type: file
    id: 019c98c8-5512-70a2-906e-7ccd03618808
    name: enriched
    inputs: [019c98c8-5512-70a2-906e-7ccd03618806]
    config:
      path: /var/lib/patrolli/flows.json
      format: json_otlp_denormalized
```
<!-- END validated config -->

The table's columns are fixed by its source: `next_hop`, `origin_as`, `peer_as`,
`local_pref`, `med`, `path_id`, `as_path`, `communities`, `large_communities`, plus the
matched `prefix`. Name any of them in `output`.

## 2. The lookup rule

- **`router`** must be bound, because each router's routes live in their own partition.
  See §3 for exporters that are not the router.
- **`vrf: global`** probes the router's default table. See §4 for VRFs.
- **`point`** is the flow's address. A list makes one rule cover IPv4 and IPv6 flows: the
  first attribute present is used.

The address attributes by flow format:

| Format | Destination | Source |
|---|---|---|
| IPFIX | `nf.data.destinationIPv4Address`, `nf.data.destinationIPv6Address` | `nf.data.sourceIPv4Address`, `nf.data.sourceIPv6Address` |
| NetFlow v9 | `nf.data.IPV4_DST_ADDR`, `nf.data.IPV6_DST_ADDR` | `nf.data.IPV4_SRC_ADDR`, `nf.data.IPV6_SRC_ADDR` |
| NetFlow v5 | `nf.data.dstaddr` | `nf.data.srcaddr` |
| sFlow | `nf.data.dst_ip` | `nf.data.src_ip` |

The exporter is always `source.address`, the UDP source of the datagram.

The key list doesn't care which format a flow came in, so one rule can cover several:

```yaml
point:
  key: [nf.data.destinationIPv4Address, nf.data.destinationIPv6Address, nf.data.IPV4_DST_ADDR, nf.data.IPV6_DST_ADDR, nf.data.dst_ip]
```

## 3. When the exporter is not the router

The route table's `router` is the address the BMP or BGP session comes from. Flows often
come from another address of the same box: a loopback against an interface address, or
a dedicated management address.

To handle this, map the exporter to its router in a table in front of the lookup, as
`map-exporters` does above. The CSV holds one row per exporter that differs:

```csv
exporter,router
192.0.2.10,198.51.100.1
192.0.2.11,198.51.100.1
```

The lookup's `router: {key: [bgp.router, source.address], cast: ip}` takes the mapped
router when the first processor wrote one, and the exporter itself otherwise. An
unmapped exporter that *is* its router needs no row.

Notes:

- **Two processors, not one.** Lookups in one processor all read the record as it came
  in, so the mapping has to happen in an earlier processor. That processor is the
  item's, and the enriching one follows it through `inputs`.
- **A mapped value must be an address.** If `bgp.router` is present but isn't an
  address, the lookup counts it as an invalid input (`lookups_errored` on the table's
  processor metrics). It does not fall back to the exporter, so a typo in the CSV shows
  up as an error rather than silently matching the wrong router.
- **Where the mapping lives.** It replaces pmacct's `bgp_agent_map` / `bmp_agent_map`.
  The CSV can live anywhere a `file` lookup table can read: it is re-read when it
  changes.

## 4. VRFs

The table holds one partition per VRF. With BMP that is per Loc-RIB instance: `global`,
or the RD as text (`65000:100`, `192.0.2.1:7`). Passive BGP always writes `global`.

A flow carries its ingress VRF only as a router-local number (IPFIX `ingressVRFID`, v9
`INGRESS_VRF_ID`). To use it:

1. Map `(exporter, VRF id)` to the RD in a table in front, the way §3 maps the router.
2. Default the rest to `global` with an insert-only `add_attributes`.
3. Bind `vrf` to the attribute instead of the literal.

```yaml
# in the mapping processor, beside the router rule
- lookup_table: <vrfs table: exporter,vrf_id,rd; type_cast: {vrf_id: integer}>
  table: record
  match:
    keys:
      exporter: {key: source.address}
      vrf_id: {key: nf.data.ingressVRFID, cast: integer}
  output:
    bgp.vrf: rd
# then a processor that defaults it
- type: add_attributes
  config:
    overwrite: false
    kv_map:
      record:
        bgp.vrf: {type: str, value: global}
# and in the route lookup
match:
  keys:
    router: {key: [bgp.router, source.address], cast: ip}
    vrf: {key: bgp.vrf}
```

Which VRFs are kept at all is the listener's `vrfs` list (empty keeps all).

## 5. Flow fields or BGP: which wins

Exporters may already fill `bgpDestinationAsNumber` and friends from their own RIB. The
recipe writes to its own keys (`dst.as`, …) and never touches `nf.data.*`, so both values
reach the sink.

To have one value instead, write the lookup into the same key the flow uses, and set the
processor's `overwrite`:

- **`overwrite: false`** (the default): the flow's value wins, and BGP only fills what
  the flow lacks.
- **`overwrite: true`**: BGP wins wherever the table has a route.

An exporter without BGP sends AS `0`, not nothing, and insert-only keeps that 0. To let
the flow win except where it says 0, use `overwrite: true` with a rule `when` that
selects the records whose AS is 0, or remove the zeros first with a `transform`.

## 6. Router configuration

These snippets are the ones `crates/app/tests/bgp_interop_e2e.rs` runs against FRR
10.7.1 and GoBGP 4.11.0. Replace `192.0.2.100` with patrolli's address.

### Passive BGP

patrolli never dials; the router connects. patrolli answers with the router's own AS
(iBGP) unless `bgp.asn` or a `peers[].asn` is set. Over iBGP, a router re-advertises
routes it learned over iBGP only to a route-reflector client.

FRR:

```text
router bgp 65020
 neighbor 192.0.2.100 remote-as 65020
 address-family ipv4 unicast
  neighbor 192.0.2.100 route-reflector-client
 exit-address-family
```

If you make the session eBGP instead (`bgp.asn` set to another AS), FRR needs
`no bgp ebgp-requires-policy` or an outbound route-map (RFC 8212), and it rewrites the
next hop to itself.

GoBGP (`remote-port` is needed only if patrolli doesn't listen on 179):

```toml
[[neighbors]]
  [neighbors.config]
    neighbor-address = "192.0.2.100"
    peer-as = 65020
  [neighbors.graceful-restart.config]
    enabled = true   # GoBGP sends End-of-RIB only with GR (patrolli advertises it)
  [[neighbors.afi-safis]]
    [neighbors.afi-safis.config]
      afi-safi-name = "ipv4-unicast"
  [[neighbors.afi-safis]]
    [neighbors.afi-safis.config]
      afi-safi-name = "ipv6-unicast"
```

### BMP Loc-RIB

FRR (`bgpd` started with `-M bmp`; Loc-RIB needs 10.0, its Peer Up 10.3):

```text
router bgp 65020
 bmp targets patrolli
  bmp connect 192.0.2.100 port 11019 min-retry 1000 max-retry 30000
  bmp monitor ipv4 unicast loc-rib
  bmp monitor ipv6 unicast loc-rib
 exit
```

GoBGP (Loc-RIB needs 4.10 for correct path ids):

```toml
[[bmp-servers]]
  [bmp-servers.config]
    address = "192.0.2.100"
    port = 11019
    route-monitoring-policy = "local-rib"
```

Junos and IOS-XR also export Loc-RIB over BMP; their configs are untested here.

### Things the interop tests found

- **One feed per router.** A router (source address) has one live session at a time. If
  it sends both BMP and BGP to the same patrolli, whichever connects second is refused
  and counted in `sessions_refused`.
- **GoBGP over BMP sends no End-of-RIB.** Routes it lost while disconnected go only when
  the BMP `keep` runs out after the reconnect.
- **GoBGP over BGP sends End-of-RIB only with Graceful Restart enabled** on its
  neighbor. FRR sends it either way.
- **FRR over BMP misses peer-loss withdrawals** (10.7.1). When one of FRR's own peers
  goes down, FRR drops the Loc-RIB withdrawals of that peer's routes
  (`bmp_wrqueue_locrib` skips peers that are no longer established). patrolli keeps
  those routes until FRR's next BMP reconnect, whose End-of-RIB sweeps them.
  Withdrawals from a peer that stays up are sent normally. Passive BGP from FRR is not
  affected.
- **FRR's Loc-RIB End-of-RIB follows each new BMP session's table sync**, and only for
  peers established at that moment. A BMP session that starts before any peer is up
  gets none.
- **FRR treats an AS_SET as withdrawn** (RFC 9774), so aggregates with sets never reach
  its Loc-RIB. GoBGP passes them on; the table shows them as `{a,b}` with a null
  `origin_as`.
- **FRR resets IPv6 over an IPv4 session without a global IPv6 address.** With IPv6
  unicast activated on an IPv4 neighbor and no global IPv6 address on the interface,
  FRR cannot choose an IPv6 next hop and resets the session with a 5/0 NOTIFICATION
  ("nexthop_set failed"). Give it an address, or carry IPv6 on an IPv6 session.

## 7. Checking it

- **Events.** The listener's events (`session_up`, `peer_up`, `end_of_rib`,
  `session_down` with any NOTIFICATION and shutdown message) reach the pipeline as log
  records whose `source.address` is the router. Give the listener an item to see them;
  `events.routes: true` adds a record per announce and withdraw.
- **Metrics.** `listener.bgp.*` per listener: `sessions_opened{protocol}`,
  `routes_announced`, `end_of_rib`, `keep_expiries`, and the `routers_live` and
  `route_table` gauges. Row counts are under `lookup_table.*`.
- **A flow that isn't enriched.** Either its router has no partition (check
  `source.address` against the mapping), or no prefix holds the address. The table's
  processor metrics count misses (`lookups_missed`) apart from invalid inputs
  (`lookups_errored`).

## 8. Limits

- **One node.** Route tables live on the node that holds the session. In a cluster,
  flows enriched on another node miss (cluster placement is phase 6 in `bgp-bmp.md`).
- **Selected routes only.** Only the router's best path per prefix is kept (Loc-RIB, or
  what the passive session receives). Adj-RIB-In is not read.
- **Memory.** About 400 bytes per route, so roughly 400 MB per full-table router.
