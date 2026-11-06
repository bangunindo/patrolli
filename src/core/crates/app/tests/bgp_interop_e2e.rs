// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Interop end to end: real routers feed the `bgp` listener, and IPFIX flows
//! come back enriched from what they sent.
//!
//! ```text
//!   GoBGP or FRR in a container
//!     ── passive BGP or BMP Loc-RIB, through the Docker host gateway ──→
//!   patrolli: bgp listener → `bgp` lookup table
//!
//!   IPFIX from 127.0.0.1 → netflow listener
//!   → map    (attribute_lookup: exporter 127.0.0.1 → the router, from a CSV)
//!   → enrich (attribute_lookup on the route table, `key: [bgp.router,
//!             source.address]`)
//!   → file sink on stdout, beside the listener's session events
//! ```
//!
//! The flows come from 127.0.0.1 and the routes from the container's address,
//! so every case goes through the exporter → router mapping: a CSV table in
//! front of the route lookup, whose key falls back from the mapped router to
//! the exporter. After each step the enriched columns
//! must equal the router's own RIB (`gobgp global rib -j`, FRR's `show bgp
//! … json`).
//!
//! Needs Docker. The router images are pinned; the containers reach patrolli
//! on the host through the Docker host gateway, so a host firewall must let
//! container → host traffic through (as for `otel_e2e_test`).

mod setup;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use setup::patrolli::{Patrolli, free_tcp_port, free_udp_port};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;
use testcontainers::core::{CmdWaitFor, ExecCommand, Host, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::net::UdpSocket;
use tokio::sync::OnceCell;
use uuid::Uuid;

/// FRR's official image. Loc-RIB monitoring needs 10.0, its Peer Up 10.3.
const FRR_IMAGE: (&str, &str) = ("quay.io/frrouting/frr", "10.7.1");
/// GoBGP publishes no image; this third-party build is kept current. Loc-RIB
/// over BMP needs 4.10 (path ids, the RFC 9069 header).
const GOBGP_IMAGE: (&str, &str) = (
    "jauderho/gobgp",
    "v4.11.0@sha256:405fd42e342da394972c314f08c6b67a1e82d7782138c26806aab6356f9c8a6e",
);

/// A debug build starting on a busy machine.
const STARTUP_LIMIT: Duration = Duration::from_secs(60);
/// Containers start, sessions come up and routers retry on their own timers.
const LIMIT: Duration = Duration::from_secs(60);

/// The AS every router here uses for itself; the passive sessions are iBGP.
const ROUTER_AS: u32 = 65020;
/// GoBGP's AS when it feeds FRR over eBGP.
const UPSTREAM_AS: u32 = 65010;

async fn wait_for<T>(what: &str, mut check: impl AsyncFnMut() -> Result<T>) -> T {
    let deadline = tokio::time::Instant::now() + LIMIT;
    loop {
        match check().await {
            Ok(value) => return value,
            Err(e) if tokio::time::Instant::now() >= deadline => {
                panic!("{what}: still failing after {LIMIT:?}: {e:#}")
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
}

async fn exec(container: &ContainerAsync<GenericImage>, cmd: &[&str]) -> Result<String> {
    let mut result = container
        .exec(ExecCommand::new(cmd.iter().copied()).with_cmd_ready_condition(CmdWaitFor::exit()))
        .await
        .with_context(|| format!("exec {cmd:?}"))?;
    let stdout = String::from_utf8_lossy(&result.stdout_to_vec().await?).into_owned();
    let code = result.exit_code().await?;
    if code != Some(0) {
        let stderr = result.stderr_to_vec().await.unwrap_or_default();
        bail!(
            "{cmd:?} exited with {code:?}: {stdout}{}",
            String::from_utf8_lossy(&stderr)
        );
    }
    Ok(stdout)
}

/// The host as the containers reach it. Router configs want an address, not
/// `host.docker.internal`, so it is resolved once inside a container.
async fn host_gateway() -> String {
    static GATEWAY: OnceCell<String> = OnceCell::const_new();
    GATEWAY
        .get_or_init(async || {
            let probe = GenericImage::new(FRR_IMAGE.0, FRR_IMAGE.1)
                .with_entrypoint("sleep")
                .with_host("host.docker.internal", Host::HostGateway)
                .with_cmd(["120"])
                .start()
                .await
                .expect("failed to start the gateway probe container");
            let out = exec(&probe, &["getent", "hosts", "host.docker.internal"])
                .await
                .unwrap();
            let gateway = out.split_whitespace().next().unwrap().to_string();
            gateway
                .parse::<IpAddr>()
                .expect("the gateway is an address");
            gateway
        })
        .await
        .clone()
}

// ---- patrolli ----

/// The ports patrolli listens on.
#[derive(Clone, Copy)]
struct Ports {
    bmp: u16,
    bgp: u16,
    netflow: u16,
    prometheus: u16,
}

/// `bmp_keep`: seconds a lost BMP session's routes stay. BGP keeps them for
/// a minute, longer than any case waits for a reconnect.
fn config(ports: Ports, exporters_csv: &std::path::Path, bmp_keep: u32) -> String {
    let Ports {
        bmp,
        bgp,
        netflow,
        prometheus,
    } = ports;
    let [
        listener,
        flows,
        routes,
        exporters,
        host,
        flow_item,
        event_item,
        map,
        enrich,
        sink,
    ] = [(); 10].map(|_| Uuid::now_v7());
    let csv = exporters_csv.display();
    format!(
        r#"global:
  logger:
    level: info
    disable_color: true
  mib:
    disabled: true
  disk_channel:
    data_dir: ./data
  internal_metrics_interval: 200
  prometheus:
    address: "127.0.0.1:{prometheus}"
lookup_tables:
  - id: {routes}
    name: routes
    source:
      type: bgp
      listener: {listener}
  - id: {exporters}
    name: exporters
    source:
      type: file
      location:
        type: file_system
      paths: [{csv}]
      reload_interval: 1
      format:
        type: csv
        has_headers: true
    key:
      type: hash
      key_columns: [exporter]
listeners:
  - id: {listener}
    name: routes
    type: bgp
    worker_count: 1
    batch_size: 1000
    batch_timeout: 100
    config:
      bmp:
        listener:
          address: 0.0.0.0:{bmp}
        keep: {bmp_keep}
      bgp:
        listener:
          address: 0.0.0.0:{bgp}
        keep: 60
  - id: {flows}
    name: flows
    type: netflow
    worker_count: 1
    batch_size: 1000
    batch_timeout: 100
    config:
      listener:
        address: 127.0.0.1:{netflow}
hosts:
  - id: {host}
    name: lab
processors:
  - id: {map}
    name: map
    type: attribute_lookup
    config:
      lookups:
        - lookup_table: {exporters}
          table: record
          match:
            keys:
              exporter: {{key: source.address}}
          output:
            bgp.router: router
  - id: {enrich}
    name: enrich
    type: attribute_lookup
    inputs: [{map}]
    config:
      lookups:
        - lookup_table: {routes}
          table: record
          match:
            keys:
              router: {{key: [bgp.router, source.address], cast: ip}}
            literals:
              vrf: global
            point:
              key: [nf.data.destinationIPv4Address, nf.data.destinationIPv6Address]
          output:
            dst.as: origin_as
            dst.as_path: as_path
            dst.network: prefix
            dst.next_hop: next_hop
            dst.communities: communities
            dst.large_communities: large_communities
            dst.med: med
            dst.local_pref: local_pref
items:
  - type: listener
    id: {flow_item}
    name: flows
    host_id: {host}
    listener_id: {flows}
    processors: [{map}]
    config:
      match_strategy:
        method: none
  - type: listener
    id: {event_item}
    name: routing-events
    host_id: {host}
    listener_id: {listener}
    processors: [{map}]
    config:
      match_strategy:
        method: none
sinks:
  - type: file
    id: {sink}
    name: stdout
    worker_count: 1
    batch_size: 1000
    batch_timeout: 100
    inputs: [{enrich}]
    config:
      path: stdout
      format: json_otlp_denormalized
"#
    )
}

/// One case's patrolli, and the flows it is probed with.
struct Bed {
    patrolli: Patrolli,
    ports: Ports,
    exporters_csv: PathBuf,
    _dir: tempfile::TempDir,
    flows: UdpSocket,
    round: u64,
}

/// What an enriched flow must carry: a value, or no such attribute at all.
/// Attributes left out are not checked.
#[derive(Debug, Clone, PartialEq)]
enum Want {
    Eq(String),
    Absent,
}

type Expect = BTreeMap<&'static str, Want>;

impl Bed {
    async fn start(bmp_keep: u32) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let exporters_csv = dir.path().join("exporters.csv");
        // a row that matches nothing until the router's address is known
        std::fs::write(&exporters_csv, "exporter,router\n192.0.2.250,192.0.2.251\n").unwrap();
        let ports = Ports {
            bmp: free_tcp_port(),
            bgp: free_tcp_port(),
            netflow: free_udp_port(),
            prometheus: free_tcp_port(),
        };
        let patrolli = Patrolli::start("interop", &config(ports, &exporters_csv, bmp_keep), &[]);
        let started = tokio::time::Instant::now();
        while !patrolli.stderr_contains("startup complete") {
            assert!(
                started.elapsed() < STARTUP_LIMIT,
                "patrolli did not start within {STARTUP_LIMIT:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let flows = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        flows.connect(("127.0.0.1", ports.netflow)).await.unwrap();
        Self {
            patrolli,
            ports,
            exporters_csv,
            _dir: dir,
            flows,
            round: 0,
        }
    }

    /// Every record on stdout, attributes flattened to dotted keys.
    fn records(&self) -> Vec<BTreeMap<String, String>> {
        self.patrolli
            .stdout_lines()
            .iter()
            .filter(|line| line.starts_with(r#"{"kind":"logs""#))
            .map(|line| {
                let value: Value = serde_json::from_str(line).unwrap();
                let mut attrs = BTreeMap::new();
                flatten("", &value["record"]["attributes"], &mut attrs);
                attrs
            })
            .collect()
    }

    /// The listener's session events of `protocol`, oldest first.
    fn events(&self, protocol: &str, event: &str) -> Vec<BTreeMap<String, String>> {
        self.records()
            .into_iter()
            .filter(|r| {
                r.get("bgp.event").map(String::as_str) == Some(event)
                    && r.get("bgp.protocol").map(String::as_str) == Some(protocol)
            })
            .collect()
    }

    /// The router of the first `protocol` session, as patrolli saw it, mapped
    /// as the exporter 127.0.0.1's router.
    async fn router(&self, protocol: &str) -> String {
        let router = wait_for(&format!("a {protocol} session"), async || {
            self.events(protocol, "session_up")
                .first()
                .and_then(|e| e.get("source.address").cloned())
                .context("no session_up yet")
        })
        .await;
        std::fs::write(
            &self.exporters_csv,
            format!("exporter,router\n127.0.0.1,{router}\n"),
        )
        .unwrap();
        router
    }

    /// Sends a flow to each destination until every one comes back enriched
    /// as `want` says.
    async fn expect(&mut self, what: &str, want: &[(&str, Expect)]) {
        let dsts: Vec<IpAddr> = want.iter().map(|(d, _)| d.parse().unwrap()).collect();
        wait_for(what, async || {
            self.round += 1;
            let round = self.round.to_string();
            self.flows.send(&ipfix(self.round, &dsts)).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            let records = self.records();
            for (dst, expect) in want {
                let flow = records
                    .iter()
                    .find(|r| {
                        r.get("nf.data.octetDeltaCount") == Some(&round)
                            && (r.get("nf.data.destinationIPv4Address").map(String::as_str)
                                == Some(dst)
                                || r.get("nf.data.destinationIPv6Address").map(String::as_str)
                                    == Some(dst))
                    })
                    .with_context(|| format!("no flow to {dst} in round {round}"))?;
                for (key, want) in expect {
                    let got = flow.get(*key);
                    let ok = match want {
                        Want::Eq(v) => got == Some(v),
                        Want::Absent => got.is_none(),
                    };
                    if !ok {
                        bail!("{dst}: {key} is {got:?}, want {want:?}");
                    }
                }
            }
            Ok(())
        })
        .await;
    }

    /// The value of a listener counter series on the Prometheus endpoint:
    /// the line of `family` whose labels contain all of `labels`.
    async fn counter(&self, family: &str, labels: &[&str]) -> Result<u64> {
        let url = format!("http://127.0.0.1:{}/metrics", self.ports.prometheus);
        let text = reqwest::get(&url).await?.text().await?;
        text.lines()
            .find(|l| {
                l.starts_with(&format!("{family}{{"))
                    && l.contains(r#"listener_name="routes""#)
                    && labels.iter().all(|label| l.contains(label))
            })
            .and_then(|l| l.rsplit(' ').next()?.parse::<f64>().ok())
            .map(|v| v as u64)
            .with_context(|| format!("no {family} {labels:?} yet"))
    }

    async fn counter_at_least(&self, family: &str, labels: &[&str], min: u64) {
        wait_for(&format!("{family} {labels:?} >= {min}"), async || {
            let n = self.counter(family, labels).await?;
            if n < min {
                bail!("{family} {labels:?} is {n}");
            }
            Ok(())
        })
        .await;
    }
}

fn flatten(prefix: &str, attrs: &Value, out: &mut BTreeMap<String, String>) {
    for kv in attrs.as_array().into_iter().flatten() {
        let Some(key) = kv["key"].as_str() else {
            continue;
        };
        let key = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        let v = &kv["value"];
        if let Some(values) = v.get("kvlistValue").map(|l| &l["values"]) {
            flatten(&key, values, out);
            continue;
        }
        let text = v["stringValue"]
            .as_str()
            .map(str::to_string)
            .or_else(|| v["intValue"].as_str().map(str::to_string))
            .or_else(|| v["intValue"].as_i64().map(|i| i.to_string()))
            .or_else(|| v["boolValue"].as_bool().map(|b| b.to_string()));
        if let Some(text) = text {
            out.insert(key, text);
        }
    }
}

// ---- IPFIX ----

fn be16(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// Appends an IPFIX set `id` holding `body`.
fn set(out: &mut Vec<u8>, id: u16, body: &[u8]) {
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&((4 + body.len()) as u16).to_be_bytes());
    out.extend_from_slice(body);
}

/// One IPFIX message, templates included: a flow to each of `dsts`, whose
/// `octetDeltaCount` is `round`, so a round's records tell themselves apart
/// from earlier ones.
fn ipfix(round: u64, dsts: &[IpAddr]) -> Vec<u8> {
    let mut sets = Vec::new();
    // 256: destinationIPv4Address/4, octetDeltaCount/8
    // 257: destinationIPv6Address/16, octetDeltaCount/8
    set(
        &mut sets,
        2,
        &be16(&[256, 2, 12, 4, 1, 8, 257, 2, 28, 16, 1, 8]),
    );
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for dst in dsts {
        match dst {
            IpAddr::V4(a) => {
                v4.extend_from_slice(&a.octets());
                v4.extend_from_slice(&round.to_be_bytes());
            }
            IpAddr::V6(a) => {
                v6.extend_from_slice(&a.octets());
                v6.extend_from_slice(&round.to_be_bytes());
            }
        }
    }
    if !v4.is_empty() {
        set(&mut sets, 256, &v4);
    }
    if !v6.is_empty() {
        set(&mut sets, 257, &v6);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32;
    let mut message = be16(&[10, (16 + sets.len()) as u16]);
    message.extend_from_slice(&now.to_be_bytes());
    message.extend_from_slice(&(round as u32).to_be_bytes());
    message.extend_from_slice(&7u32.to_be_bytes());
    message.extend_from_slice(&sets);
    message
}

// ---- reference RIB ----

/// A route as a router's RIB shows it, in the table's text forms.
#[derive(Debug, Clone, Default)]
struct Route {
    prefix: String,
    /// `(is_set, asns)` per segment
    segments: Vec<(bool, Vec<u32>)>,
    communities: Vec<String>,
    large_communities: Vec<String>,
    med: Option<u32>,
    local_pref: Option<u32>,
    next_hop: Option<String>,
}

impl Route {
    fn as_path(&self) -> String {
        self.segments
            .iter()
            .map(|(is_set, asns)| {
                let asns: Vec<String> = asns.iter().map(u32::to_string).collect();
                if *is_set {
                    format!("{{{}}}", asns.join(","))
                } else {
                    asns.join(" ")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The last AS of the path; none for an aggregate ending in a set of
    /// several.
    fn origin_as(&self) -> Option<u32> {
        match self.segments.last()? {
            (false, asns) => asns.last().copied(),
            (true, asns) if asns.len() == 1 => Some(asns[0]),
            (true, _) => None,
        }
    }

    /// The flow attributes this route must produce.
    fn expect(&self) -> Expect {
        let mut e = Expect::new();
        e.insert("dst.network", Want::Eq(self.prefix.clone()));
        e.insert(
            "dst.as",
            self.origin_as()
                .map_or(Want::Absent, |a| Want::Eq(a.to_string())),
        );
        if !self.segments.is_empty() {
            e.insert("dst.as_path", Want::Eq(self.as_path()));
        }
        if !self.communities.is_empty() {
            e.insert("dst.communities", Want::Eq(self.communities.join(" ")));
        }
        if !self.large_communities.is_empty() {
            e.insert(
                "dst.large_communities",
                Want::Eq(self.large_communities.join(" ")),
            );
        }
        if let Some(med) = self.med {
            e.insert("dst.med", Want::Eq(med.to_string()));
        }
        if let Some(lp) = self.local_pref {
            e.insert("dst.local_pref", Want::Eq(lp.to_string()));
        }
        if let Some(nh) = &self.next_hop {
            e.insert("dst.next_hop", Want::Eq(nh.clone()));
        }
        e
    }
}

/// A flow whose destination no route covers.
fn unrouted() -> Expect {
    BTreeMap::from([("dst.network", Want::Absent), ("dst.as", Want::Absent)])
}

// ---- GoBGP ----

struct GoBgp(ContainerAsync<GenericImage>);

impl GoBgp {
    async fn start(toml: String, network: Option<&str>) -> Self {
        let image = GenericImage::new(GOBGP_IMAGE.0, GOBGP_IMAGE.1)
            .with_wait_for(WaitFor::message_on_either_std(
                "Finished reading the config file",
            ))
            .with_copy_to("/config/gobgp.toml", toml.into_bytes());
        let container = match network {
            Some(network) => image.with_network(network).start().await,
            None => image.start().await,
        }
        .expect("failed to start GoBGP");
        Self(container)
    }

    async fn gobgp(&self, args: &[&str]) -> String {
        let cmd: Vec<&str> = std::iter::once("gobgp")
            .chain(args.iter().copied())
            .collect();
        // gobgpd answers on its API a moment after it logs its config
        wait_for(&format!("{cmd:?}"), async || exec(&self.0, &cmd).await).await
    }

    async fn add(&self, family: &str, prefix: &str, attrs: &[&str]) {
        let mut args = vec!["global", "rib", "add", "-a", family, prefix];
        args.extend_from_slice(attrs);
        self.gobgp(&args).await;
    }

    async fn del(&self, family: &str, prefix: &str) {
        self.gobgp(&["global", "rib", "del", "-a", family, prefix])
            .await;
    }

    /// `prefix`'s best path in GoBGP's global RIB.
    async fn route(&self, family: &str, prefix: &str) -> Route {
        let json: Value =
            serde_json::from_str(&self.gobgp(&["global", "rib", "-j", "-a", family]).await)
                .unwrap();
        let path = json[prefix]
            .as_array()
            .and_then(|paths| paths.iter().find(|p| p["best"] == true))
            .unwrap_or_else(|| panic!("{prefix} is not in GoBGP's RIB: {json}"));
        let mut route = Route {
            prefix: prefix.to_string(),
            ..Route::default()
        };
        for attr in path["attrs"].as_array().unwrap() {
            match attr["type"].as_u64().unwrap() {
                2 => {
                    for seg in attr["as_paths"].as_array().unwrap() {
                        let asns = seg["asns"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|a| a.as_u64().unwrap() as u32)
                            .collect();
                        route.segments.push((seg["segment_type"] == 1, asns));
                    }
                }
                3 | 14 => route.next_hop = attr["nexthop"].as_str().map(str::to_string),
                4 => route.med = attr["metric"].as_u64().map(|v| v as u32),
                5 => route.local_pref = attr["value"].as_u64().map(|v| v as u32),
                8 => {
                    route.communities = attr["communities"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|c| {
                            let c = c.as_u64().unwrap() as u32;
                            format!("{}:{}", c >> 16, c & 0xffff)
                        })
                        .collect()
                }
                32 => {
                    route.large_communities = attr["value"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|c| format!("{}:{}:{}", c["ASN"], c["LocalData1"], c["LocalData2"]))
                        .collect()
                }
                _ => {}
            }
        }
        route
    }
}

/// GoBGP as the router: `neighbor` is patrolli over iBGP (with GR, so it
/// sends End-of-RIB), `bmp` a BMP station it reports its Loc-RIB to.
fn gobgp_config(gateway: &str, bgp_port: Option<u16>, bmp_port: Option<u16>) -> String {
    let mut toml =
        format!("[global.config]\n  as = {ROUTER_AS}\n  router-id = \"10.255.0.1\"\n  port = -1\n");
    if let Some(port) = bgp_port {
        toml += &format!(
            r#"
[[neighbors]]
  [neighbors.config]
    neighbor-address = "{gateway}"
    peer-as = {ROUTER_AS}
  [neighbors.transport.config]
    remote-port = {port}
  [neighbors.timers.config]
    connect-retry = 1
  [neighbors.graceful-restart.config]
    enabled = true
  [[neighbors.afi-safis]]
    [neighbors.afi-safis.config]
      afi-safi-name = "ipv4-unicast"
  [[neighbors.afi-safis]]
    [neighbors.afi-safis.config]
      afi-safi-name = "ipv6-unicast"
"#
        );
    }
    if let Some(port) = bmp_port {
        toml += &format!(
            r#"
[[bmp-servers]]
  [bmp-servers.config]
    address = "{gateway}"
    port = {port}
    route-monitoring-policy = "local-rib"
"#
        );
    }
    toml
}

/// The routes every GoBGP case injects, and the flow destination each
/// covers.
const GOBGP_ROUTES: &[(&str, &str, &[&str], &str)] = &[
    (
        "ipv4",
        "203.0.113.0/24",
        &[
            "aspath",
            "10 20 {30,40}",
            "nexthop",
            "192.0.2.1",
            "community",
            "65000:1,65000:2",
            "large-community",
            "65000:1:2",
            "med",
            "10",
            "local-pref",
            "150",
        ],
        "203.0.113.9",
    ),
    (
        "ipv4",
        "203.0.0.0/16",
        &["aspath", "10 23", "nexthop", "192.0.2.1"],
        "203.0.9.9",
    ),
    (
        "ipv4",
        "198.51.100.0/24",
        &["aspath", "10 22", "nexthop", "192.0.2.2"],
        "198.51.100.9",
    ),
    (
        "ipv6",
        "2001:db8:5::/48",
        &[
            "aspath",
            "10 21",
            "nexthop",
            "2001:db8::1",
            "community",
            "65000:6",
        ],
        "2001:db8:5::9",
    ),
];

async fn gobgp_inject(router: &GoBgp) {
    for (family, prefix, attrs, _) in GOBGP_ROUTES {
        router.add(family, prefix, attrs).await;
    }
}

/// Each GoBGP route's flow, expected as GoBGP's RIB has it.
async fn gobgp_expect(router: &GoBgp) -> Vec<(&'static str, Expect)> {
    let mut want = Vec::new();
    for (family, prefix, _, dst) in GOBGP_ROUTES {
        want.push((*dst, router.route(family, prefix).await.expect()));
    }
    want
}

// ---- FRR ----

struct Frr(ContainerAsync<GenericImage>);

const FRR_DAEMONS: &str = "bgpd=yes
vtysh_enable=yes
zebra_options=\"  -A 127.0.0.1 -s 90000000\"
mgmtd_options=\"  -A 127.0.0.1\"
bgpd_options=\"   -A 127.0.0.1 -M bmp\"
staticd_options=\"-A 127.0.0.1\"
";

impl Frr {
    async fn start(conf: String, network: Option<&str>) -> Self {
        let image = GenericImage::new(FRR_IMAGE.0, FRR_IMAGE.1)
            .with_wait_for(WaitFor::message_on_either_std("bgpd state -> up"))
            .with_copy_to("/etc/frr/daemons", FRR_DAEMONS.as_bytes().to_vec())
            .with_copy_to("/etc/frr/vtysh.conf", Vec::new())
            .with_copy_to("/etc/frr/frr.conf", conf.into_bytes())
            // zebra and bgpd refuse to start without them
            .with_cap_add("NET_ADMIN")
            .with_cap_add("SYS_ADMIN");
        let container = match network {
            Some(network) => image.with_network(network).start().await,
            None => image.start().await,
        }
        .expect("failed to start FRR");
        Self(container)
    }

    async fn vtysh(&self, commands: &[&str]) -> String {
        let mut cmd = vec!["vtysh"];
        for c in commands {
            cmd.extend(["-c", c]);
        }
        exec(&self.0, &cmd).await.unwrap()
    }

    /// Runs `lines` under `router bgp`.
    async fn configure(&self, lines: &[&str]) {
        let router = format!("router bgp {ROUTER_AS}");
        let mut commands = vec!["configure terminal", router.as_str()];
        commands.extend_from_slice(lines);
        self.vtysh(&commands).await;
    }

    /// `prefix`'s best path in FRR's RIB, once it has one.
    async fn route(&self, family: &str, prefix: &str) -> Route {
        let show = format!("show bgp {family} unicast {prefix} json");
        wait_for(&show, async || {
            let json: Value = serde_json::from_str(&self.vtysh(&[&show]).await)?;
            let path = json["paths"]
                .as_array()
                .and_then(|paths| paths.iter().find(|p| p["bestpath"]["overall"] == true))
                .with_context(|| format!("{prefix} has no best path in FRR: {json}"))?;
            let segments = path["aspath"]["segments"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|seg| {
                    let asns = seg["list"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_u64().unwrap() as u32)
                        .collect();
                    (seg["type"] == "as-set", asns)
                })
                .collect();
            let list = |field: &str| {
                path[field]["list"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| c.as_str().unwrap().to_string())
                    .collect()
            };
            Ok(Route {
                prefix: prefix.to_string(),
                segments,
                communities: list("community"),
                large_communities: list("largeCommunity"),
                med: path["metric"].as_u64().map(|v| v as u32),
                local_pref: path["locPrf"].as_u64().map(|v| v as u32),
                next_hop: path["nexthops"][0]["ip"].as_str().map(str::to_string),
            })
        })
        .await
    }
}

// ---- cases ----

/// GoBGP dials patrolli over iBGP; its routes, withdrawals and a restart's
/// End-of-RIB reach the table.
#[tokio::test]
async fn gobgp_passive_bgp() {
    setup::setup_logging();
    let mut bed = Bed::start(60).await;
    let gateway = host_gateway().await;
    let router = GoBgp::start(gobgp_config(&gateway, Some(bed.ports.bgp), None), None).await;
    let peer = bed.router("bgp").await;

    gobgp_inject(&router).await;
    bed.expect("GoBGP's routes", &gobgp_expect(&router).await)
        .await;

    // a withdraw: the flow falls back to the covering /16
    router.del("ipv4", "203.0.113.0/24").await;
    let covering = router.route("ipv4", "203.0.0.0/16").await.expect();
    bed.expect("the /24 withdrawn", &[("203.0.113.9", covering)])
        .await;

    // a route deleted while the session is down goes with the next
    // End-of-RIB; the others stay through the gap
    router.gobgp(&["neighbor", &gateway, "disable"]).await;
    wait_for("the session to go down", async || {
        let down = bed.events("bgp", "session_down");
        let down = down.first().context("no session_down yet")?;
        assert_eq!(
            down.get("bgp.notification").map(String::as_str),
            Some("6/2")
        );
        Ok(())
    })
    .await;
    router.del("ipv4", "198.51.100.0/24").await;
    let end_of_rib = bed
        .counter("patrolli_listener_bgp_end_of_rib_total", &[])
        .await
        .unwrap_or(0);
    router.gobgp(&["neighbor", &gateway, "enable"]).await;
    bed.counter_at_least(
        "patrolli_listener_bgp_end_of_rib_total",
        &[],
        end_of_rib + 2,
    )
    .await;
    let mut want = gobgp_expect_without(&router, "198.51.100.0/24").await;
    want.push(("198.51.100.9", unrouted()));
    bed.expect("the End-of-RIB sweep", &want).await;

    bed.counter_at_least(
        "patrolli_listener_bgp_sessions_opened_total",
        &[r#"protocol="bgp""#],
        2,
    )
    .await;
    assert_eq!(
        bed.counter("patrolli_listener_bgp_keep_expiries_total", &[])
            .await
            .unwrap_or(0),
        0,
        "the End-of-RIB, not `keep`, swept the deleted route"
    );
    assert!(
        bed.events("bgp", "session_up")
            .iter()
            .all(|e| e["source.address"] == peer)
    );
    bed.patrolli.stop().await;
}

/// The GoBGP routes still in its RIB, expected.
async fn gobgp_expect_without(router: &GoBgp, gone: &str) -> Vec<(&'static str, Expect)> {
    let mut want = Vec::new();
    for (family, prefix, _, dst) in GOBGP_ROUTES {
        if *prefix != gone && *prefix != "203.0.113.0/24" {
            want.push((*dst, router.route(family, prefix).await.expect()));
        }
    }
    want
}

/// GoBGP reports its Loc-RIB over BMP, path ids included; it sends no
/// End-of-RIB, so a route lost while the session was down goes when `keep`
/// runs out.
#[tokio::test]
async fn gobgp_bmp_loc_rib() {
    setup::setup_logging();
    let mut bed = Bed::start(2).await;
    let gateway = host_gateway().await;
    let router = GoBgp::start(gobgp_config(&gateway, None, Some(bed.ports.bmp)), None).await;
    bed.router("bmp").await;

    gobgp_inject(&router).await;
    bed.expect("GoBGP's Loc-RIB", &gobgp_expect(&router).await)
        .await;

    router.del("ipv4", "203.0.113.0/24").await;
    let covering = router.route("ipv4", "203.0.0.0/16").await.expect();
    bed.expect("the /24 withdrawn", &[("203.0.113.9", covering)])
        .await;

    // the session drops and a route goes while it is down; the reconnect
    // dumps the others again. Not a container restart: Docker may hand the
    // container another address, and routes are keyed by the session's.
    let station = format!("{gateway}:{}", bed.ports.bmp);
    router.gobgp(&["bmp", "del", &station]).await;
    wait_for("the session to go down", async || {
        bed.events("bmp", "session_down")
            .first()
            .map(|_| ())
            .context("no session_down yet")
    })
    .await;
    router.del("ipv4", "198.51.100.0/24").await;
    router.gobgp(&["bmp", "add", &station, "local-rib"]).await;
    bed.counter_at_least(
        "patrolli_listener_bgp_sessions_opened_total",
        &[r#"protocol="bmp""#],
        2,
    )
    .await;
    let mut want = gobgp_expect_without(&router, "198.51.100.0/24").await;
    want.push(("198.51.100.9", unrouted()));
    bed.expect("the stale route gone with `keep`", &want).await;
    bed.counter_at_least("patrolli_listener_bgp_keep_expiries_total", &[], 1)
        .await;
    bed.patrolli.stop().await;
}

fn frr_config(gateway: &str, bgp_port: u16, router_id: &str) -> String {
    format!(
        "frr defaults traditional
hostname r1
log stdout informational
!
route-map ORIGIN permit 10
 set as-path prepend 65001 65002
 set community 65000:1 65000:2
 set large-community 65000:1:2
 set metric 10
 set local-preference 150
exit
!
router bgp {ROUTER_AS}
 bgp router-id {router_id}
 no bgp ebgp-requires-policy
 no bgp network import-check
 neighbor {gateway} remote-as {ROUTER_AS}
 neighbor {gateway} port {bgp_port}
 neighbor {gateway} timers connect 1
 address-family ipv4 unicast
  network 203.0.113.0/24 route-map ORIGIN
  network 203.0.0.0/16
 exit-address-family
exit
"
    )
}

/// FRR dials patrolli over iBGP: route-map attributes, a withdrawal, an
/// End-of-RIB without Graceful Restart, and an RFC 9003 shutdown message.
#[tokio::test]
async fn frr_passive_bgp() {
    setup::setup_logging();
    let mut bed = Bed::start(60).await;
    let gateway = host_gateway().await;
    let router = Frr::start(frr_config(&gateway, bed.ports.bgp, "10.255.0.2"), None).await;
    let peer = bed.router("bgp").await;

    // FRR shows its own routes with no next hop; over iBGP it sends itself
    let mut v4 = router.route("ipv4", "203.0.113.0/24").await;
    v4.next_hop = Some(peer.clone());
    let mut covering = router.route("ipv4", "203.0.0.0/16").await;
    covering.next_hop = Some(peer.clone());
    bed.expect(
        "FRR's routes",
        &[
            ("203.0.113.9", v4.expect()),
            ("203.0.9.9", covering.expect()),
        ],
    )
    .await;
    // sent on a plain session: GR is not what makes FRR send it
    bed.counter_at_least("patrolli_listener_bgp_end_of_rib_total", &[], 1)
        .await;

    router
        .configure(&["address-family ipv4 unicast", "no network 203.0.113.0/24"])
        .await;
    bed.expect("the /24 withdrawn", &[("203.0.113.9", covering.expect())])
        .await;

    router
        .configure(&[&format!("neighbor {gateway} shutdown message maintenance")])
        .await;
    let down = wait_for("FRR's shutdown", async || {
        bed.events("bgp", "session_down")
            .into_iter()
            .next()
            .context("no session_down yet")
    })
    .await;
    assert_eq!(
        down.get("bgp.notification").map(String::as_str),
        Some("6/2")
    );
    assert_eq!(
        down.get("bgp.message").map(String::as_str),
        Some("maintenance")
    );
    // the routes outlive the session for `keep`
    bed.expect("routes kept", &[("203.0.113.9", covering.expect())])
        .await;
    bed.patrolli.stop().await;
}

/// GoBGP feeds FRR over eBGP, and FRR reports its Loc-RIB over BMP: learned
/// routes, an upstream withdrawal, and a BMP reconnect whose End-of-RIB
/// sweeps what was lost meanwhile.
///
/// Not covered: the upstream session going down. FRR 10.7.1 drops the queued
/// Loc-RIB withdrawals of a peer that is no longer established
/// (`bmp_wrqueue_locrib` in `bgpd/bgp_bmp.c`), so its routes stay in the
/// table until the next BMP reconnect, whose End-of-RIB sweeps them.
#[tokio::test]
async fn frr_bmp_loc_rib() {
    setup::setup_logging();
    let mut bed = Bed::start(60).await;
    let gateway = host_gateway().await;
    let network = format!("bgp-interop-{}", &Uuid::now_v7().simple().to_string()[..8]);
    let bmp_port = bed.ports.bmp;
    let router = Frr::start(
        format!(
            "frr defaults traditional
hostname r2
log stdout informational
!
router bgp {ROUTER_AS}
 bgp router-id 10.255.0.4
 no bgp ebgp-requires-policy
 neighbor UPSTREAM peer-group
 neighbor UPSTREAM remote-as {UPSTREAM_AS}
 bgp listen range 0.0.0.0/0 peer-group UPSTREAM
 bmp targets patrolli
  bmp connect {gateway} port {bmp_port} min-retry 100 max-retry 1000
  bmp monitor ipv4 unicast loc-rib
 exit
exit
"
        ),
        Some(&network),
    )
    .await;
    bed.router("bmp").await;
    let frr = router.0.get_bridge_ip_address().await.unwrap();
    let upstream_config = format!(
        r#"[global.config]
  as = {UPSTREAM_AS}
  router-id = "10.255.0.5"
  port = -1

[[neighbors]]
  [neighbors.config]
    neighbor-address = "{frr}"
    peer-as = {ROUTER_AS}
  [neighbors.timers.config]
    connect-retry = 1
"#
    );
    let upstream = GoBgp::start(upstream_config, Some(&network)).await;
    // FRR treats an AS_SET as withdrawn (RFC 9774), so the paths are plain
    // sequences here (the GoBGP cases cover the set), and without a next hop
    // GoBGP gives its own, one FRR can reach
    let v4: Vec<(&str, &str, Vec<&str>, &str)> = GOBGP_ROUTES
        .iter()
        .filter(|r| r.0 == "ipv4")
        .map(|(family, prefix, attrs, dst)| {
            let mut plain = Vec::new();
            let mut args = attrs.iter();
            while let Some(arg) = args.next() {
                match *arg {
                    "nexthop" => {
                        args.next();
                    }
                    "10 20 {30,40}" => plain.push("10 20 30"),
                    arg => plain.push(arg),
                }
            }
            (*family, *prefix, plain, *dst)
        })
        .collect();
    for (family, prefix, attrs, _) in &v4 {
        upstream.add(family, prefix, attrs).await;
    }
    let mut want = Vec::new();
    for (family, prefix, _, dst) in &v4 {
        want.push((*dst, router.route(family, prefix).await.expect()));
    }
    // learned over eBGP: the upstream's AS leads the path
    assert_eq!(
        want[0].1["dst.as_path"],
        Want::Eq(format!("{UPSTREAM_AS} 10 20 30"))
    );
    bed.expect("FRR's Loc-RIB", &want).await;

    upstream.del("ipv4", "203.0.113.0/24").await;
    let covering = router.route("ipv4", "203.0.0.0/16").await.expect();
    bed.expect("the /24 withdrawn upstream", &[("203.0.113.9", covering)])
        .await;

    // FRR sends a Loc-RIB End-of-RIB only after the table sync of a new BMP
    // session, once per peer then established, so a reconnect with the
    // upstream up gets one; it sweeps a route lost while BMP was down
    let connect = format!("bmp connect {gateway} port {bmp_port} min-retry 100 max-retry 1000");
    router
        .configure(&["bmp targets patrolli", &format!("no {connect}")])
        .await;
    wait_for("the BMP session to go down", async || {
        bed.events("bmp", "session_down")
            .first()
            .cloned()
            .context("no session_down yet")
    })
    .await;
    upstream.del("ipv4", "198.51.100.0/24").await;
    router.configure(&["bmp targets patrolli", &connect]).await;
    bed.counter_at_least("patrolli_listener_bgp_end_of_rib_total", &[], 1)
        .await;
    let mut want = Vec::new();
    for (family, prefix, _, dst) in &v4 {
        if !["203.0.113.0/24", "198.51.100.0/24"].contains(prefix) {
            want.push((*dst, router.route(family, prefix).await.expect()));
        }
    }
    want.push(("198.51.100.9", unrouted()));
    bed.expect("the End-of-RIB sweep", &want).await;
    assert_eq!(
        bed.counter("patrolli_listener_bgp_keep_expiries_total", &[])
            .await
            .unwrap_or(0),
        0
    );
    bed.patrolli.stop().await;
}
