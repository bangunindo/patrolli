# BGP/BMP for patrolli — research notes (2026-11-04)

Goal: a listener that accepts both BMP and passive BGP sessions and keeps their routes in a
lookup table, which enriches netflow/IPFIX/sFlow records with src/dst AS, AS path, communities,
prefix length and BGP next hop. Section 6 is the implementation plan.

These are design notes gathered before implementation, not a description of shipped
behavior. Versions and figures are as of the date above.

## References

BMP:
- [RFC 7854](https://www.rfc-editor.org/rfc/rfc7854) BMP
- [RFC 8671](https://www.rfc-editor.org/rfc/rfc8671) Adj-RIB-Out
- [RFC 9069](https://www.rfc-editor.org/rfc/rfc9069) Loc-RIB
- [RFC 9736](https://www.rfc-editor.org/rfc/rfc9736) Peer Up TLV namespace
- [RFC 9972](https://www.rfc-editor.org/rfc/rfc9972) more stats types
- [draft-ietf-grow-bmp-tlv](https://datatracker.ietf.org/doc/draft-ietf-grow-bmp-tlv/) (BMPv4)
- [draft-ietf-grow-bmp-path-marking-tlv](https://datatracker.ietf.org/doc/draft-ietf-grow-bmp-path-marking-tlv/)
- [IANA BMP parameters](https://www.iana.org/assignments/bmp-parameters/)

BGP:
- [RFC 4271](https://www.rfc-editor.org/rfc/rfc4271) BGP-4
- [RFC 4760](https://www.rfc-editor.org/rfc/rfc4760) MP-BGP
- [RFC 6793](https://www.rfc-editor.org/rfc/rfc6793) 4-octet ASN
- [RFC 7911](https://www.rfc-editor.org/rfc/rfc7911) ADD-PATH
- [RFC 4724](https://www.rfc-editor.org/rfc/rfc4724) End-of-RIB
- [RFC 8654](https://www.rfc-editor.org/rfc/rfc8654) extended message
- [RFC 7606](https://www.rfc-editor.org/rfc/rfc7606) error handling
- [RFC 8950](https://www.rfc-editor.org/rfc/rfc8950) v4 NLRI with v6 next hop
- [RFC 4456](https://www.rfc-editor.org/rfc/rfc4456) route reflection
- [RFC 6286](https://www.rfc-editor.org/rfc/rfc6286) BGP identifier

Communities and VPN:
- [RFC 1997](https://www.rfc-editor.org/rfc/rfc1997) communities
- [RFC 4360](https://www.rfc-editor.org/rfc/rfc4360) extended communities
- [RFC 8092](https://www.rfc-editor.org/rfc/rfc8092) large communities
- [RFC 4364](https://www.rfc-editor.org/rfc/rfc4364) L3VPN
- [RFC 4659](https://www.rfc-editor.org/rfc/rfc4659) L3VPN IPv6

Other implementations:
- [akvorado BMP provider](https://github.com/akvorado/akvorado/tree/main/outlet/routing/provider/bmp) (read at `f77463c`)
- [pmacct CONFIG-KEYS](https://github.com/pmacct/pmacct/blob/master/CONFIG-KEYS) and [bmp_lookup.c](https://github.com/pmacct/pmacct/blob/master/src/bmp/bmp_lookup.c)
- [NetGauze](https://github.com/NetGauze/NetGauze)

---

## 1. Protocol essentials

### BMP framing (RFC 7854)
- TCP, router → station only. The station never sends anything; to restart it just
  drops the connection. Router usually *active* (connects to us); retry backoff
  30s → 720s, so a dropped session costs minutes of RIB.
- **Ports**: BMP has no assigned port (§3.2: configured on both ends). akvorado defaults to
  10179 (verified in its `bmp/config.go`); pmacct 1790 and OpenBMP 5000 (from memory, not
  verified). BGP is fixed at TCP 179 (RFC 4271 §4). That port is privileged: it needs
  `CAP_NET_BIND_SERVICE`, or a high port with the remote port set on the router. BMP and BGP
  can't share a socket: BMP opens with version byte 3, BGP with a 16-byte all-ones marker.
- Common header: `version u8 (=3; 4 = draft-ietf-grow-bmp-tlv) | length u32 (incl.
  header) | type u8`. Unknown types MUST be ignored → length-delimited skip is safe.
- Types: 0 Route Monitoring, 1 Stats, 2 Peer Down, 3 Peer Up, 4 Initiation,
  5 Termination, 6 Route Mirroring.
- Per-peer header (42 B) on all but Initiation/Termination:
  `peer_type u8 | flags u8 | peer_distinguisher u64 | peer_addr 16B | peer_as u32 |
  peer_bgp_id u32 | ts_sec u32 | ts_usec u32`.
  - peer types: 0 Global, 1 RD instance (PD = RD), 2 Local instance, 3 Loc-RIB (RFC 9069)
  - flags (types 0-2): `V`(0x80) IPv6 peer, `L`(0x40) post-policy, `A`(0x20) legacy 2-byte
    AS_PATH, `O`(0x10, RFC 8671) Adj-RIB-Out. Type 3: `F`(0x80) filtered Loc-RIB.
  - "Peer Type + Peer Distinguisher disambiguate peers whose other identity overlaps."
- Lifecycle: Initiation (MUST be first; sysDescr + sysName TLVs) → Peer Up per established
  peer → Route Monitoring dump → End-of-RIB per peer → incremental updates. Dump and
  incremental updates interleave; a withdraw for an unknown route is normal ("bogus
  withdraw, safely ignore").
- Peer Up: per-peer hdr + local addr (16B) + local port + remote port + **sent OPEN** +
  **received OPEN** + TLVs (RFC 9736 gave Peer Up its own TLV namespace; type 3 =
  VRF/Table name from RFC 9069, type 4 = admin label). Capabilities in the two OPENs decide
  how to decode that peer's UPDATEs: 4-octet ASN, ADD-PATH per AFI/SAFI (direction matters:
  Adj-RIB-In = remote SEND ∧ local RECEIVE; Adj-RIB-Out = local SEND ∧ remote RECEIVE),
  multiple labels.
- Peer Down: reasons 1-5 (+6 = Loc-RIB instance down, RFC 9069). **Implicitly withdraws
  every route of that peer**; router may skip explicit withdraws.
- Route Monitoring: per-peer hdr + one BGP UPDATE PDU. State-compressed (not verbatim);
  pre- and post-policy streams multiplexed by the L flag; a withdraw carries the same L as
  the announce it cancels.
- Stats: counters/gauges; 7/8 routes in Adj-RIB-In / Loc-RIB, 9/10 per AFI/SAFI,
  14-17 Adj-RIB-Out (RFC 8671), 18-43 RFC 9972. Good for "does our RIB match the router".
- Termination: reason TLV (0 admin close, 1 unspecified, 2 out of resources, 3 redundant,
  4 permanently closed). Must close after it. Connection can also just die.
- Route Mirroring: verbatim PDUs, possibly malformed. Every enrichment collector ignores it.
- Multiple BGP instances on one router ⇒ multiple BMP sessions, told apart by sysName / string TLV.
- Security: no auth in BMP; restrict by source IP (allow-list) and size-cap messages.

### RIB views

BGP is one protocol, but a router keeps several route tables (RIBs, RFC 4271 §3.2):

```
neighbour A ─► Adj-RIB-In (A) ─┐
neighbour B ─► Adj-RIB-In (B) ─┼─► import policy ─► best-path selection ─► Loc-RIB
neighbour C ─► Adj-RIB-In (C) ─┘                                             │
                                                     ┌───────────────────────┤
                                                     ▼                       ▼
                                          export policy (X)         export policy (Y)
                                                     │                       │
                                                     ▼                       ▼
                                           Adj-RIB-Out (X)          Adj-RIB-Out (Y)
                                                     │                       │
                                                     ▼                       ▼
                                                neighbour X             neighbour Y
```

- **Adj-RIB-In**, one per neighbour: everything it announced. A prefix appears once per neighbour.
- **Loc-RIB**, one per router (per VRF): one selected best path per prefix, the one traffic follows.
- **Adj-RIB-Out**, one per neighbour: what the router announces after that neighbour's export policy.
- The FIB (forwarding table) is not BGP; it is built from the Loc-RIB and other sources, and may
  hold several next hops under ECMP.

BMP can stream five views, told apart by peer type and the L/O flags: Adj-RIB-In pre- and
post-policy (RFC 7854), Loc-RIB (RFC 9069), Adj-RIB-Out pre- and post-policy (RFC 8671).
"Pre-policy" Adj-RIB-Out is the per-neighbour selection before export policy, not a separate table.

A passive BGP session receives the router's **Adj-RIB-Out toward patrolli**: normally its best
paths, filtered by the export policy facing us. Two catches:
- **iBGP does not re-advertise iBGP-learned routes** to other iBGP peers (RFC 4456 lifts this),
  so patrolli must be configured as a **route-reflector client** to see the whole table.
- **eBGP rewrites the next hop** to the router itself; iBGP keeps the original.

For flow enrichment only one path per prefix per router is useful: the one the flow followed.
Loc-RIB and passive BGP give exactly that. Adj-RIB-In gives one candidate per neighbour and needs
best-path selection, or akvorado's next-hop match, to pick one. Every BMP router sends Adj-RIB-In;
Loc-RIB support is newer and depends on vendor and OS version.

### Loc-RIB (RFC 9069) — the most useful feed for enrichment
- Peer type 3, peer addr zero-filled, peer AS = router's AS, BGP ID = router-id of the VRF.
- Carries the router's *selected* best paths (incl. locally originated), so no best-path
  computation on our side. PD = 0 for the global table, else the VRF's RD; Peer Up TLV 3 names
  the table ("global" by default).
- Always 4-octet ASN; fabricated OPEN (sent == received) declares ADD-PATH if used.
- One VRF may come as several emulated peers (one per AF): key by (PD, BGP ID), merge
  capabilities.

### BGP UPDATE details the decoder must get right
- Max size 4096, or 65535 with Extended Message (RFC 8654): never assume 4 KiB.
- ADD-PATH (RFC 7911): 4-byte path id prepended to each NLRI; only knowable from the capabilities
  exchange (the Peer Up OPENs). Heuristics (bgpkit-parser) are fragile.
- 4-octet ASN (RFC 6793): if the A flag is set / no 4-octet capability, AS_PATH is 2-byte and
  AS4_PATH must be merged: if |AS_PATH| < |AS4_PATH| ignore AS4_PATH; else take the leading
  (|AS_PATH| − |AS4_PATH|) ASNs of AS_PATH + AS4_PATH. AS_TRANS = 23456.
- MP-BGP (RFC 4760): v6 and VPN routes only in MP_REACH/MP_UNREACH (attr 14/15); next hop from
  MP_REACH (v6 may be global+link-local 32 B; RFC 8950 v4-over-v6 next hop).
- End-of-RIB (RFC 4724): v4 unicast = empty UPDATE; other AF = UPDATE with only an empty
  MP_UNREACH for that AFI/SAFI.
- Error handling (RFC 7606): prefer treat-as-withdraw / attribute-discard over killing the
  session; for a monitoring station, count and skip.
- Attributes worth keeping: ORIGIN(1), AS_PATH(2), NEXT_HOP(3), MED(4), LOCAL_PREF(5),
  COMMUNITIES(8, RFC 1997), MP_REACH/UNREACH(14/15), EXT_COMMUNITIES(16, RFC 4360; route
  targets), AS4_PATH(17), LARGE_COMMUNITIES(32, RFC 8092). ORIGINATOR_ID/CLUSTER_LIST (RFC 4456)
  matter only for RR setups.
- VPN (RFC 4364/4659): VPNv4/v6 NLRI = label(s) + 8-byte RD + prefix.

---

## 2. How others do flow ↔ RIB correlation

| | RIB scope | Exporter → RIB | Path choice |
|---|---|---|---|
| **akvorado** | one global RIB for all routers, peers, VRFs | **ignored** | path whose next hop == flow next hop, else first stored ("somewhat approximate") |
| **pmacct** | per BGP/BMP peer | flow source IP == BMP session IP / router-id, or `bmp_agent_map`; also "reflector" case (agent == a monitored peer inside a BMP session) | RD must match; with ADD-PATH, next hop must equal flow's BGP next hop |
| **Kentik** | per device (iBGP session, device = RR client) | device | device's best path |
| **akvorado BioRIS** | per router (external RIB) | router-id == agent IP, else arbitrary | longest prefix, `Paths[0]` |

### akvorado specifics worth copying
- Read loop and decode loop split by a bounded channel (`message-buffer`, 10k): routers
  disconnect a slow station, and a peer flush is slow. Full channel ⇒ TCP backpressure + counter.
- `SO_RCVBUF` configurable; read back and warn if the kernel capped it (`net.core.rmem_max`).
- Max BMP message 1 MiB (2026.10 security fix); first message must be Initiation.
- Bad UPDATE (GoBGP ErrorHandling): skip and count `ignored_updates_total{error}`; non-parse
  failures kill the connection.
- RIB: one prefix trie (`bart`) behind an atomic pointer, persistent insert/delete under one
  writer mutex (RCU); routes in 16 RwLock shards keyed `prefix_idx<<32 | route_idx`; route ≈
  20 B with `u32` refs into refcounted intern pools (attrs tuple = origin AS + AS path +
  communities + large communities interned together; next hop separately). Generation counter
  on recycled prefix indices so a reader with an old trie snapshot never reads a reused slot.
- Peer key = (exporter addr:**port**, peer IP, peer type, PD, peer AS, BGP ID). AFI/SAFI and L/O
  flags not in key (pre+post for the same peer collide).
- Connection loss ⇒ peers marked stale for `keep` (5m) and still served; Peer Down ⇒ flush
  immediately. Flush = O(whole RIB) under global write lock (documented perf problem).
  Reconnect from a new source port creates new peers; old ones expire by `keep`.
- Flags: `collect-asns`, `collect-aspaths`, `collect-communities`; `rds`/`rts` filters (only way
  to keep VRFs apart since lookup ignores RD).
- Enrichment ordering: `asn-providers: [flow, routing, networks]` (also `flow-except-private`,
  `flow-except-default-route`, `routing-except-private`); `net-providers: [flow, routing]` for
  netmask + next hop. First non-zero wins. Private = 0, 23456, 64496-65551, ≥ 4.2e9.
- Source side: lookup on src IP with no next hop (assumes symmetric routing — pmacct warns
  about the same).
- Their latent bug to avoid: a zero/absent next hop counted as "specified" so routing's
  next hop never wins.
- Vendor configs in akvorado `console/data/docs/11-exporters.md`: all post-policy; Arista
  also `bgp rib bestpaths` (Loc-RIB); IOS XR needs ≥ 24.2.1; don't half-close (XR closes too).
- Metrics: connections opened/closed, peers, routes, received_messages{type}, errors{error},
  ignored_updates{error}, ignored_nlri{type}, queue full, prefixes added/removed/updated,
  locked_duration{peer-removal}, lookups / failed lookups.

---

## 3. Rust building blocks (from the crate survey, partly spot-checked)

Parsers:
- **netgauze-bmp-pkt / -bgp-pkt 0.13** (Apache-2.0, edition 2024, active): BMP v3+v4 TLVs,
  RFC 8671 O flag, RFC 9069 F flag, VRF TLVs, stats. `BmpCodec` (tokio-util `Decoder`) keeps a
  per-peer decode context from Peer Up, with ADD-PATH direction for Adj-RIB-In, Adj-RIB-Out and
  Loc-RIB handled correctly (verified in netgauze-bmp-pkt 0.13.0 `src/codec.rs`). Owned/allocating model; RFC 7606
  partial; pre-1.0 churn. **Front-runner.**
- routecore 0.7 (NLnet Labs, BSD-3): zero-copy lazy views, FSM feature; possible panic on a
  malformed 2nd Peer Up TLV (unbounds-checked slice; from code reading); 4096-byte caps in helpers.
- bgpkit-parser 0.23 (MIT): great for MRT/tests, but BMP route monitoring hard-codes ADD-PATH
  off and uses a heuristic. Not for live BMP.
- zettabgp: stats/mirroring unparsed, raw flags. bgp-rs / bmp-protocol: dead.

LPM / RIB:
- **prefix-trie 0.10** (already a dependency, `cache::lookup_table::prefix`): tree-bitmap; on a
  1.04M v4 / 246k v6 table ~23 Mops/s v4 LPM; clone of a full v4 table ~8-10 ms; `mem_size()`
  O(1). No internal concurrency ⇒ RCU via ArcSwap, batched swaps.
- rotonda-store: lock-free but never deletes nodes and allocates per query — wrong for a hot path.
- iptrie (LC-trie, rebuild-and-swap), poptrie (too young), ip_network_table (unmaintained).

### What NetGauze covers for passive BGP + BMP

Read from the 0.13.0 sources; none of it has been run.

| Crate | Scope | Use it? |
|---|---|---|
| `netgauze-bmp-pkt` | BMP parse/encode; `BmpCodec` (`codec` feature) carries per-peer decode context from Peer Up | **Yes**, under our own TCP listener |
| `netgauze-bgp-pkt` | BGP message types (OPEN, UPDATE, KEEPALIVE, NOTIFICATION, ROUTE-REFRESH) parse **and** serialize; `BgpCodec` encodes and decodes | **Yes**, for both BMP's embedded UPDATEs and the passive BGP session |
| `netgauze-bgp-speaker` | RFC 4271 FSM: OPEN exchange, keepalive and hold timers, NOTIFICATION, passive `BgpListener` (registered or dynamic peers), collision handling, UPDATE events tagged with RFC 7606 treatment | **No** (reasons below) |
| `netgauze-bmp-service` | BMP TCP server, actor/supervisor, SO_REUSEPORT, subscribe API | **No** (reasons below) |

The packet crates have no session logic: no FSM, no timers, nothing sent on their own.
- **BMP needs none.** It is one-way: no handshake, no keepalive, nothing to send back.
- **Passive BGP needs a small session.** `bgp-pkt` provides every message, but timers and
  state are ours.

Why not `netgauze-bgp-speaker`:
- **ADD-PATH would be decoded wrongly.** `BgpCodec` learns only 4-octet ASN from the OPEN
  exchange; neither the codec nor the speaker has any ADD-PATH handling (zero references). The
  sample `EchoCapabilitiesPolicy` echoes the peer's capabilities back, so a router offering
  ADD-PATH gets it negotiated and every NLRI after that is misparsed. A custom `PeerPolicy` that
  never offers ADD-PATH avoids this.
- **Unbounded `mpsc` channels** between peer, controller and consumer: a full-table dump is
  buffered in memory with no backpressure on the router.
- **Interpolated log messages** (`"[{}][{}] ..."`) defeat patrolli's throttle (see the Logging
  section of CLAUDE.md); its target would need filtering.
- **General-purpose FSM** (active connect, collision detection, damping) whose own task tree
  would sit beside patrolli's supervisor and shutdown ordering rather than inside them.

Why not `netgauze-bmp-service`:
- **It drops messages.** A subscriber slower than a timeout loses messages (counted in
  `subscriber_dropped`). For a RIB, a lost withdraw is a silently wrong route until the session
  restarts. We should stop reading instead and let TCP backpressure reach the router (akvorado).
- **Metrics through OpenTelemetry** instead of `InternalMetrics`.

### Plan: own transports over the NetGauze packet crates

- **BMP**: `FramedRead<TcpStream, BmpCodec>` in a listener transport (`ensure_started`,
  supervisor, `JoinSet`). Reader and decoder separated by a bounded channel, message-size cap,
  source-IP allow-list.
- **Passive BGP** (a few hundred lines, on `Framed<TcpStream, BgpCodec>`):
  1. Accept on the configured port and check the source against configured peers.
  2. Read the peer's OPEN and validate peer AS and hold time.
  3. Reply with our OPEN (4-octet ASN; no ADD-PATH unless the codec's `BgpParsingContext`
     is set up for it via `new_from_ctx`) and a KEEPALIVE.
  4. Run two timers: send a KEEPALIVE every hold/3; send NOTIFICATION (hold timer expired)
     and close if nothing arrives within the hold time.
  5. Pass UPDATEs to the RIB; on any error, send NOTIFICATION and close.
- **Shared**: BGP message types, and both sources write the same kind of route table
  (section 6).

---

## 4. Patrolli integration points

- Netflow rows (`listener::netflow`): logs; exporter = `LogRow.source_ip`; fields under
  `nf.data.<IANA name>`: `sourceIPv4Address`/`destinationIPv4Address` (+v6),
  `bgpNextHopIPv4Address`/`bgpNextHopIPv6Address`, `ipNextHop*`, `bgpSourceAsNumber`,
  `bgpDestinationAsNumber`, `source/destinationIPv4PrefixLength` (+v6), community lists.
  sFlow: `nf.data.src_ip`/`dst_ip`, `extended_gateway.{as,src_as,src_peer_as,dst_as_path,
  communities,next_hop,local_pref}`, `extended_router.{next_hop,src_mask_len,dst_mask_len}`.
- Existing generic machinery (since `3270eb2`): mutable prefix-keyed lookup tables with
  partition columns (`cache::lookup_table::mutable::upsert_prefix`), longest-prefix match per
  partition, exact-prefix `remove`, and `remove_partition` (O(1) unlink, freed outside the
  lock: Peer Down). Long strings (> 23 bytes) are pooled per shard. Read by `attribute_lookup`
  (sync, hot path safe), which can also output the matched prefix. This is where routes live
  (section 6), with these caveats:
  - `attribute_collect` must not write routes: "batches on different processor workers are not
    ordered against each other", so a withdraw could land before its announce.
  - A column or key edit (`reuses_rows` false) rebuilds the table empty. File tables reload;
    a BMP router never resends its table without a reconnect.
  - Expiry is by TTL on `last_seen`; stable routes are never re-announced, so a route table
    needs TTL 0 plus a separate stale sweep.
  - One row per (partition, prefix): fits Loc-RIB and passive BGP, not several neighbours'
    Adj-RIB-In in one partition (one peer's withdraw would delete another's route).
  - No row interning: each prefix owns an `Arc<MutableRow>` and its cells. Estimated (not
    measured) 200–300 B per route plus the trie node, ~300 MB per full-table router.
  - A partition lives in one shard, so a full dump write-locks that shard ~1M times.
  - `max_rows` is table-wide (default 1M); a rejected prefix makes lookups fall back to a wider
    prefix, i.e. wrong answers rather than missing ones.
- Listener transport conventions (CLAUDE.md): transport owned by the listener, started
  lazily by `ensure_started`, `Idle | Running | Closed`, survives worker restarts; producers own
  the Sender; supervisor respawn with backoff; no panics on peer input (fuzz the decode path).
- Cluster: a router's BMP TCP session lands on whichever node it connects to; its netflow may be
  processed on the node owning its host (`listener::annotate` forward split). RIB placement must
  follow the same host ownership or be replicated.

---

## 5. Open design decisions

1. ~~RIB scope~~ **Decided: per router**, as partitions of the route table (pmacct's model).
   Still open: the exporter → router mapping when the BMP/BGP address differs from the netflow
   source (a hash lookup table in front, or a host-config override).
2. ~~Which RIB to keep~~ **Decided for v1: Loc-RIB and passive BGP.** Adj-RIB-In is counted and
   skipped; supporting it (next-hop match or path selection) is a later phase.
3. ~~Where enrichment runs~~ **Decided: in `cache::lookup_table`**, through a new `bgp` table
   source owned by the listener (section 6). No `cache::route`, no new lookup processor.
4. **VRF/RD**: partition by PD/RD, map flow → VRF (ingress interface? VRF id IE 234/235).
5. ~~Stale handling~~ **Decided**: Peer Down → `remove_partition`; connection loss → keep
   serving for `keep`; after the new dump's End-of-RIB (or when `keep` runs out) sweep what
   the dump did not refresh. Mechanics in section 6, phase 1.
6. **Provider precedence**: flow-supplied AS vs routing vs lookup tables (akvorado's ordered
   providers).
7. ~~Passive BGP in addition to BMP?~~ **Decided: both.** Passive BGP covers routers without
   BMP (Kentik's model); see "Plan" in section 3. Still open: whether to ever negotiate
   ADD-PATH on the passive session.
8. **Cluster placement** of route tables (phase 6).
9. **Listener shape**: one `bgp` listener type with a BMP port and a BGP port, or two types
   sharing the table source. Decide in phase 3.
10. **Row interning**: only if the phase 1 bench shows memory per route is a problem.

---

## 6. Implementation plan

### Route table: a `bgp` lookup table source

Routes go into a mutable prefix lookup table whose source is a new `cfg::Source` variant beside
`File` and `Processor`, naming the listener that fills it (name and fields provisional):

```yaml
lookup_tables:
  - name: routes
    source:
      type: bgp
      listener: <listener id>
      max_rows: 4000000     # table-wide; size for every router's full table
    # no columns and no key: both are fixed by the source
```

- **Columns and key come from the source**, not the user: key `prefix` partitioned by router and
  table/VRF; columns such as origin AS, AS path, communities, large communities, next hop,
  local pref, MED. Users cannot get them wrong. Possibly let the source drop attribute groups
  (akvorado's `collect-aspaths`/`collect-communities`) to save memory.
- **Only the listener writes it.** `attribute_collect` refuses a `bgp` table, so pipeline
  writes can never interleave with a session's stream.
- **TTL is always 0**; staleness is handled by the sweep below.
- **Rebuilds are visible.** The source names its listener, so when a config edit rebuilds the
  table, the listener knows it must resync.
- `attribute_lookup` reads it like any other prefix table: probe with the flow's address and the
  router's partition, output the columns it wants and optionally the matched prefix.

### Session writes the table; flume carries only pipeline output

The session task (one per BMP or BGP connection) applies UPDATEs, Peer Up/Down and End-of-RIB to
the table **itself, in stream order**. The flume channel to the listener's workers only carries
optional route events as log records, best effort (drop and count when full). Two reasons:
- **Order**: workers reading one channel can apply a withdraw before its announce. Neither BGP
  nor BMPv3 has sequence numbers to repair that; TCP stream order is the only order.
- **Backpressure**: if routes went through workers that also feed the processors, a stuck sink
  would stop route updates, fill the TCP buffers, and get the BMP session dropped by the router.

### Phases

1. **Lookup table groundwork** (no network code)
   - `bgp` source variant: config, validation, fixed schema and key, `attribute_collect`
     refusing it, API and `openapi.json`, `config-full.yaml`.
   - A stale sweep: remove the rows of one partition whose `last_seen` is older than a cutoff.
     Upserts already stamp `last_seen`, so cutoff = session start.
   - Rebuild detection: a generation on `Mutable`, or the session comparing its `Arc`, so a
     session whose table was replaced reconnects (BMP) or sends ROUTE-REFRESH (BGP, if
     negotiated) to get a fresh dump.
   - `bench_rib`: load a RIPE RIS MRT dump (bgpkit-parser as a dev-dependency) into a prefix
     table; measure memory per route, dump rate, and lookup latency while writing. Decides
     whether row interning is needed now.
2. **BMP session**
   - `FramedRead<TcpStream, BmpCodec>`; Initiation must come first; message size cap; source
     allow-list.
   - Peer Up/Down; keep Loc-RIB, count and skip Adj-RIB-In.
   - UPDATE → table writes; End-of-RIB → sweep; disconnect → `keep` grace, then sweep.
   - Unit tests from captured BMP bytes; fuzz the decode path (no panics on peer input).
3. **Listener wiring**
   - Transport through `ensure_started`, supervisor, `InternalMetrics` (akvorado's metric set
     in section 2 is a good start).
   - Listener config, backend API, `config-full.yaml`.
   - Optional route events into the pipeline over flume, best effort.
4. **Passive BGP**
   - OPEN validation, KEEPALIVE every hold/3, hold timer, NOTIFICATION on errors (section 3).
   - Same write path as BMP; optional TCP-MD5.
   - Document the route-reflector-client requirement for iBGP.
5. **Enrichment and e2e**
   - Document the `attribute_lookup` recipe, including the exporter → router mapping.
   - Docker e2e: FRR or GoBGP feeding routes plus a flow generator; assert enriched records.
6. **Cluster placement**: mutable tables are per node. Either flows from a router are enriched on
   the node holding its session, or the routes are replicated.

Later, only if needed: Adj-RIB-In (next-hop match as akvorado, or path selection), row interning,
BMPv4 TLVs (including the Sequence Number TLV).

### Testing

Cheapest layers first. Layers 1 and 3 with crafted messages give the most confidence per hour;
GoBGP as a reference RIB is the first e2e to write.

1. **Unit, no network** (phases 1–2)
   - Lookup table additions: stale sweep, rebuild detection, `bgp` source validation,
     `attribute_collect` refusing a `bgp` table. Same style as the `mutable.rs` tests.
   - Session logic on **crafted messages**: NetGauze serializes as well as parses, so tests build
     exact byte streams. Cover what captures rarely contain: 2-byte AS_PATH + AS4_PATH merge,
     one-directional ADD-PATH, Extended Message (> 4096 B), End-of-RIB per address family,
     withdraw of an unknown prefix, Peer Down without explicit withdraws, no Initiation first,
     oversized message, malformed attributes (RFC 7606).
   - `cargo fuzz` on decode + apply: peer input must never panic (panics are fatal here).
2. **Captured bytes** (phase 2)
   - akvorado `outlet/routing/provider/bmp/testdata/`: 13 small captures from a **Juniper vMX**
     (init, peer-up/peers-up, peer-down, reach/unreach, add-path, Loc-RIB, L3VPN, EoR, VPLS,
     unknown family, terminate). Their test writes each capture's TCP payload into a socket in
     sequence. akvorado is AGPL-3.0; copy with attribution, after checking "-only" vs
     "-or-later" against our AGPL-3.0-only.
   - Read them by extending the pcap reader in `tests/bench_netflow_replay.rs` (`pcap_udp`) to
     TCP payloads, or convert once to raw `.bin` streams.
   - Not checked: whether NetGauze's repository or Wireshark's sample captures include BMP.
   - Gap: nearly all public captures are Juniper. Add IOS-XR, Arista, Nokia captures as fixtures
     when a lab or customer router is available.
3. **Transport, real sockets, no Docker** (phases 2–4)
   - BMP: the test connects to the listener, writes captured or crafted bytes, checks the table
     with `setup::eventually`.
   - Passive BGP: the test plays the router with NetGauze's `BgpCodec`, with short hold times
     (3 s is the minimum non-zero). Cases: OPEN exchange; KEEPALIVE cadence; hold-timer expiry →
     NOTIFICATION and close; peer not on the allow-list rejected; ADD-PATH never negotiated;
     mid-stream disconnect → routes kept for `keep`, then swept.
   - A config edit that rebuilds the table forces a reconnect.
4. **Interop e2e with GoBGP and FRR in Docker** (phase 5). Images pinned; patrolli reached on the
   host through the Docker host gateway, as `otel_e2e_test` does.
   - **GoBGP**, the most scriptable:
     - BMP client (`bmp-servers`) dialing patrolli. Its Loc-RIB monitoring option is believed
       to exist but not verified.
     - Active peer for the passive BGP session.
     - `gobgp global rib add/del` injects and withdraws exact prefixes from the test.
     - **Reference RIB**: after each step, compare the patrolli table with `gobgp global rib -j`.
     - `gobgp mrt inject` loads a whole RIS dump for a full-table e2e.
   - **FRR**, closer to a real router: BMP via `bgpd -M bmp` (`loc-rib` monitoring in recent
     versions; the version that added it is unverified), route-reflector-client setup for
     passive iBGP, realistic Peer Up/Down sequences when its neighbours restart.
   - Probe enrichment without a NetFlow generator, as `tests/lookup_table_e2e.rs` does: OTLP log
     records carrying a destination IP and the exporter/router attribute go through
     `attribute_lookup`, and the enriched output is read from a file sink on stdout. One case can
     use a NetFlow packet from the replay pcap for realism.
5. **Scale**: public MRT dumps, RIPE RIS (`bview`, e.g. rrc00) and RouteViews. Loaded in-process
   with bgpkit-parser (dev-dependency) for `bench_rib`, and through GoBGP for the full-table e2e.
6. **In production**: a self-check metric. BMP Stats Reports carry the router's own route counts
   (types 7/8, Adj-RIB-In and Loc-RIB); expose them beside our per-partition row counts. A
   persistent gap means something was lost or misapplied, which no test environment catches.
