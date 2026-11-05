// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// End-to-end test: configuration made through the REST API reaches a running
// patrolli node. Listeners, sinks, hosts, endpoints and items are each inserted,
// disabled, enabled and deleted, and every step is checked by what the node
// does, from outside the process:
//
// - a listener: its TCP port accepts connections, and it has a Prometheus series
// - a sink: its Prometheus series, and the syslog tokens that reach its file
// - a host and its listener item: a token's record carries the item's id
//   (`patrolli.item.id`) when matched, and reaches the catch-all sink without it
//   when not
// - an endpoint and its SNMP item: a fake agent the test owns gets polled
//
// Both sinks start with no inputs, so they take every record no processor
// claims. The item routes to no processor, which keeps `DELETE /hosts` clear of
// N26 (`item_processors` has no ON DELETE CASCADE). The endpoint is deleted only
// with its host: `DELETE /endpoints` fails while an item still points at it.
//
// One node, in backend mode, run from the binary cargo built for this test. It
// serves the API itself, so this goes through the handlers' SQL and the
// triggers as an operator would.

mod setup;

use anyhow::{Context, Result, bail, ensure};
use parking_lot::Mutex;
use reqwest::Method;
use serde_json::{Value, json};
use setup::eventually;
use setup::patrolli::{Patrolli, free_tcp_port};
use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::info;
use uuid::Uuid;

const JWT_SECRET: &str = "eiK4eu4hie2oolaiph8eej2eehooNeig";
const ADMIN_USER: &str = "admin";
const ADMIN_PASSWORD: &str = "e2e-admin-password";
/// How long the fake agent has to go unpolled for its items to count as
/// stopped. Their schedule is every second.
const QUIET: Duration = Duration::from_secs(3);

struct Node {
    patrolli: Patrolli,
    api: String,
    prometheus: String,
    http: reqwest::Client,
    token: String,
}

impl Node {
    async fn start(db_url: &str) -> Result<Self> {
        let api_port = free_tcp_port();
        let prometheus_port = free_tcp_port();
        let inter_port = free_tcp_port();
        let node_id = Uuid::now_v7();
        let config = format!(
            r#"
global:
  internal_metrics_interval: 200
  logger:
    level: info
    disable_color: true
  mib:
    disabled: true
  prometheus:
    address: "127.0.0.1:{prometheus_port}"
  disk_channel:
    data_dir: ./data
  backend:
    db_url: "{db_url}"
    watcher_interval: 1
    api:
      addr: "127.0.0.1:{api_port}"
    cluster:
      node_id: {node_id}
      node_name: node-1
      heartbeat_interval: 1
      heartbeat_timeout: 3
      metadata:
        advertise_address: "127.0.0.1:{inter_port}"
      inter_node:
        grpc:
          address: "127.0.0.1:{inter_port}"
"#
        );
        let patrolli = Patrolli::start(
            "node-1",
            &config,
            &[
                ("PATROLLI_JWT_SECRET", JWT_SECRET),
                ("PATROLLI_ADMIN_USER", ADMIN_USER),
                ("PATROLLI_ADMIN_PASSWORD", ADMIN_PASSWORD),
            ],
        );
        let mut node = Self {
            patrolli,
            api: format!("http://127.0.0.1:{api_port}"),
            prometheus: format!("http://127.0.0.1:{prometheus_port}/metrics"),
            http: reqwest::Client::new(),
            token: String::new(),
        };
        node.token = eventually(async || node.login().await).await;
        Ok(node)
    }

    async fn login(&self) -> Result<String> {
        let response = self
            .http
            .post(format!("{}/api/v1/auth/login", self.api))
            .json(&json!({ "username": ADMIN_USER, "password": ADMIN_PASSWORD }))
            .send()
            .await
            .context("the API is not up yet")?;
        let status = response.status();
        let body: Value = response.json().await?;
        ensure!(status.is_success(), "login: {status} {body}");
        body["data"]["access_token"]
            .as_str()
            .map(str::to_string)
            .context("login answered without an access token")
    }

    /// Calls the API and hands back the response's `data`.
    async fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let mut request = self
            .http
            .request(method.clone(), format!("{}/api/v1{path}", self.api))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("{method} {path}"))?;
        let status = response.status();
        let text = response.text().await?;
        ensure!(status.is_success(), "{method} {path}: {status} {text}");
        if text.is_empty() {
            return Ok(Value::Null);
        }
        let body: Value = serde_json::from_str(&text)?;
        Ok(body["data"].clone())
    }

    /// Creates a resource and returns its id, read from the `id_field` of the
    /// response.
    async fn create(&self, path: &str, body: &Value, id_field: &str) -> Result<Uuid> {
        let data = self.call(Method::POST, path, Some(body)).await?;
        let id = data[id_field]
            .as_str()
            .with_context(|| format!("POST {path} answered without {id_field}: {data}"))?;
        Ok(id.parse()?)
    }

    /// `PUT`s `body` back with its `adm_status` set to `adm_status`.
    async fn set_adm_status(&self, path: &str, body: &mut Value, adm_status: i16) -> Result<()> {
        body["adm_status"] = json!(adm_status);
        self.call(Method::PUT, path, Some(body)).await?;
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<()> {
        self.call(Method::DELETE, path, None).await?;
        Ok(())
    }

    /// Whether the node reports a `metric` series labelled `label="id"`. A
    /// listener or sink has one exactly while the node runs it.
    async fn has_series(&self, metric: &str, label: &str, id: Uuid) -> Result<bool> {
        let body = self
            .http
            .get(&self.prometheus)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .context("failed to scrape the node")?
            .text()
            .await?;
        let series = format!("{metric}{{");
        let label = format!("{label}=\"{id}\"");
        Ok(body
            .lines()
            .any(|l| l.starts_with(&series) && l.contains(&label)))
    }
}

/// Waits until the node runs (`running`) or no longer runs the sink `id`.
async fn wait_sink(node: &Node, id: Uuid, running: bool) {
    eventually(async || {
        let has = node
            .has_series("patrolli_sink_workers", "sink_id", id)
            .await?;
        ensure!(has == running, "sink {id} running: {has}, want {running}");
        Ok(())
    })
    .await
}

/// Waits until the node runs the syslog listener `id` on `port`, or stopped
/// it: its series and its socket both have to agree.
async fn wait_listener(node: &Node, id: Uuid, port: u16, running: bool) {
    eventually(async || {
        let has = node
            .has_series("patrolli_listener_workers", "listener_id", id)
            .await?;
        let accepts = TcpStream::connect(("127.0.0.1", port)).is_ok();
        ensure!(
            has == running && accepts == running,
            "listener {id} running: {has}, accepting on {port}: {accepts}, want {running}"
        );
        Ok(())
    })
    .await
}

/// Lines of a file sink's output, empty while it has written nothing.
fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn line_with(path: &Path, token: &str) -> Option<String> {
    lines(path).into_iter().find(|l| l.contains(token))
}

/// One syslog message carrying `token`, over its own TCP connection from
/// 127.0.0.1.
fn send_syslog(port: u16, token: &str) -> Result<()> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    writeln!(
        stream,
        "<134>1 2026-10-20T00:00:00Z e2e-host e2e-app 1 ID47 - {token}"
    )?;
    Ok(())
}

fn new_token() -> String {
    format!("e2e-token-{}", Uuid::now_v7().simple())
}

/// Sends a fresh token and returns it with the line the control sink wrote
/// for it.
async fn route(syslog_port: u16, control: &Path) -> Result<(String, String)> {
    let token = new_token();
    send_syslog(syslog_port, &token)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if let Some(line) = line_with(control, &token) {
            return Ok((token, line));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("the control sink never wrote {token}")
}

/// Waits until a token sent now comes out matched to `item_id` or, with
/// `matched` false, comes out with no item at all.
async fn wait_matched(syslog_port: u16, control: &Path, item_id: Uuid, matched: bool) {
    let item_id = item_id.to_string();
    eventually(async || {
        let (_, line) = route(syslog_port, control).await?;
        ensure!(
            line.contains(&item_id) == matched,
            "want matched to item {item_id}: {matched}, got {line}"
        );
        if !matched {
            ensure!(
                !line.contains("patrolli.item.id"),
                "want no item at all, got {line}"
            );
        }
        Ok(())
    })
    .await
}

/// An SNMP agent that answers every request with the varbinds it was asked
/// for, all NULL, and notes when it was asked. It has to answer: a check that
/// times out is retried only every 30 seconds, far apart from its schedule.
struct Agent {
    port: u16,
    polls: Arc<Mutex<Vec<Instant>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Agent {
    async fn start() -> Result<Self> {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await?;
        let port = socket.local_addr()?.port();
        let polls = Arc::new(Mutex::new(Vec::new()));
        let seen = polls.clone();
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            while let Ok((n, from)) = socket.recv_from(&mut buf).await {
                seen.lock().push(Instant::now());
                if let Some(pdu) = pdu_offset(&buf[..n]) {
                    // GetRequest, GetNextRequest or GetBulkRequest → Response
                    buf[pdu] = 0xa2;
                    _ = socket.send_to(&buf[..n], from).await;
                }
            }
        });
        Ok(Self { port, polls, task })
    }

    /// Waits until it is polled after `since`.
    async fn wait_polled(&self, since: Instant) {
        eventually(async || {
            ensure!(
                self.polls.lock().iter().any(|t| *t > since),
                "not polled since the change"
            );
            Ok(())
        })
        .await
    }

    /// Waits until nothing has polled it for [`QUIET`].
    async fn wait_quiet(&self) {
        eventually(async || {
            let last = self.polls.lock().last().copied();
            if let Some(last) = last {
                ensure!(last.elapsed() >= QUIET, "polled {:?} ago", last.elapsed());
            }
            Ok(())
        })
        .await
    }
}

/// Where the PDU starts in an SNMP v1/v2c message: past the message
/// sequence's header, the version and the community.
fn pdu_offset(msg: &[u8]) -> Option<usize> {
    /// The offset just past a BER length that starts at `at`, and the length.
    fn length(msg: &[u8], at: usize) -> Option<(usize, usize)> {
        let first = *msg.get(at)? as usize;
        if first < 0x80 {
            return Some((at + 1, first));
        }
        let n = first & 0x7f;
        let bytes = msg.get(at + 1..at + 1 + n)?;
        Some((
            at + 1 + n,
            bytes.iter().fold(0, |len, b| len << 8 | *b as usize),
        ))
    }
    let (mut at, _) = length(msg, 1)?;
    for _ in 0..2 {
        let (content, len) = length(msg, at + 1)?;
        at = content + len;
    }
    matches!(msg.get(at)?, 0xa0 | 0xa1 | 0xa5).then_some(at)
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn file_sink(name: &str, path: &Path, adm_status: i16) -> Value {
    json!({
        "name": name,
        "adm_status": adm_status,
        "labels": {},
        "worker_count": 1,
        "inputs": [],
        "config": {
            "type": "file",
            "config": { "path": path.display().to_string(), "format": "json_otlp_denormalized" },
        },
        "batch": { "batch_size": 1000, "batch_timeout": 100 },
        "channel": { "type": "memory" },
    })
}

#[tokio::test]
async fn api_changes_reach_the_running_node() -> Result<()> {
    setup::setup_logging();
    let (db, _pool) = setup::setup_db().await;
    let dir = tempfile::tempdir()?;
    let control: PathBuf = dir.path().join("control.jsonl");
    let toggled: PathBuf = dir.path().join("toggled.jsonl");
    let syslog_port = free_tcp_port();
    let agent = Agent::start().await?;
    let mut node = Node::start(&db.url).await?;

    // ---- Inserted disabled: nothing runs --------------------------------
    // The control sink is created last, so once it runs, the node has read
    // everything before it.
    let mut listener = json!({
        "name": "syslog-tcp",
        "adm_status": 0,
        "labels": {},
        "worker_count": 1,
        "batch": { "batch_size": 1000, "batch_timeout": 100 },
        "config": {
            "type": "syslog",
            "config": { "listener": { "tcp": { "address": format!("127.0.0.1:{syslog_port}") } } },
        },
    });
    let listener_id = node.create("/listeners", &listener, "listener_id").await?;
    let listener_path = format!("/listeners/{listener_id}");
    let mut toggled_sink = file_sink("toggled", &toggled, 0);
    let toggled_id = node.create("/sinks", &toggled_sink, "sink_id").await?;
    let toggled_path = format!("/sinks/{toggled_id}");
    let control_id = node
        .create("/sinks", &file_sink("control", &control, 1), "sink_id")
        .await?;
    info!("waiting for the control sink");
    wait_sink(&node, control_id, true).await;
    assert!(
        !node
            .has_series("patrolli_sink_workers", "sink_id", toggled_id)
            .await?,
        "a sink inserted disabled must not run"
    );
    assert!(
        !node
            .has_series("patrolli_listener_workers", "listener_id", listener_id)
            .await?
            && TcpStream::connect(("127.0.0.1", syslog_port)).is_err(),
        "a listener inserted disabled must not run"
    );

    // ---- Listener enabled: records flow, matched to nothing --------------
    info!("enabling the listener");
    node.set_adm_status(&listener_path, &mut listener, 1)
        .await?;
    wait_listener(&node, listener_id, syslog_port, true).await;
    let (_, line) = eventually(async || route(syslog_port, &control).await).await;
    assert!(
        !line.contains("patrolli.item.id"),
        "no host yet, so no item: {line}"
    );

    // ---- Host with a listener item and an SNMP item ----------------------
    info!("creating the host and its items");
    let mut host = json!({ "name": "e2e-host", "adm_status": 1, "labels": {} });
    let host_id = node.create("/hosts", &host, "host_id").await?;
    let host_path = format!("/hosts/{host_id}");
    let mut listener_item = json!({
        "listener_id": listener_id,
        "endpoint_id": null,
        "flags": 0,
        "adm_status": 1,
        "name": "syslog",
        "schedule": "never",
        "timeout_ms": 10000,
        "labels": {},
        "config": {
            "type": "listener",
            "config": { "match_strategy": { "method": "source_ips", "config": ["127.0.0.1"] } },
        },
    });
    let listener_item_id = node
        .create(&format!("/items/host/{host_id}"), &listener_item, "item_id")
        .await?;
    let listener_item_path = format!("/items/{listener_item_id}/host");
    let mut endpoint = json!({
        "host_id": host_id,
        "name": "agent",
        "adm_status": 1,
        "config": {
            "type": "snmp_v2c",
            "config": { "address": format!("127.0.0.1:{}", agent.port), "community": "public" },
        },
    });
    let endpoint_id = node.create("/endpoints", &endpoint, "endpoint_id").await?;
    let endpoint_path = format!("/endpoints/{endpoint_id}");
    let mut snmp_item = json!({
        "listener_id": null,
        "endpoint_id": endpoint_id,
        "flags": 0,
        "adm_status": 1,
        "name": "sys-uptime",
        "schedule": "1",
        "timeout_ms": 500,
        "labels": {},
        "config": {
            "type": "snmp",
            "config": { "oid": "1.3.6.1.2.1.1.3.0", "use_mib": false },
        },
    });
    let snmp_item_id = node
        .create(&format!("/items/host/{host_id}"), &snmp_item, "item_id")
        .await?;
    let snmp_item_path = format!("/items/{snmp_item_id}/host");
    wait_matched(syslog_port, &control, listener_item_id, true).await;
    agent.wait_polled(Instant::now()).await;

    // ---- Sink: enable, disable (N3), enable ------------------------------
    info!("enabling the toggled sink");
    node.set_adm_status(&toggled_path, &mut toggled_sink, 1)
        .await?;
    wait_sink(&node, toggled_id, true).await;
    let token = new_token();
    send_syslog(syslog_port, &token)?;
    eventually(async || {
        ensure!(
            line_with(&toggled, &token).is_some(),
            "the enabled sink has not written {token}"
        );
        Ok(())
    })
    .await;

    info!("disabling the toggled sink");
    node.set_adm_status(&toggled_path, &mut toggled_sink, 0)
        .await?;
    wait_sink(&node, toggled_id, false).await;
    // routes drop a sink before its workers stop, so nothing sent from here
    // on can reach it
    let (token, _) = route(syslog_port, &control).await?;
    assert!(
        line_with(&toggled, &token).is_none(),
        "a disabled sink must not write"
    );

    info!("enabling the toggled sink again");
    node.set_adm_status(&toggled_path, &mut toggled_sink, 1)
        .await?;
    wait_sink(&node, toggled_id, true).await;

    // ---- Listener item: disable, enable ----------------------------------
    info!("disabling the listener item");
    node.set_adm_status(&listener_item_path, &mut listener_item, 0)
        .await?;
    wait_matched(syslog_port, &control, listener_item_id, false).await;
    info!("enabling the listener item");
    node.set_adm_status(&listener_item_path, &mut listener_item, 1)
        .await?;
    wait_matched(syslog_port, &control, listener_item_id, true).await;

    // ---- SNMP item: disable, enable --------------------------------------
    info!("disabling the snmp item");
    node.set_adm_status(&snmp_item_path, &mut snmp_item, 0)
        .await?;
    agent.wait_quiet().await;
    info!("enabling the snmp item");
    let since = Instant::now();
    node.set_adm_status(&snmp_item_path, &mut snmp_item, 1)
        .await?;
    agent.wait_polled(since).await;

    // ---- Endpoint: disable, enable ---------------------------------------
    info!("disabling the endpoint");
    node.set_adm_status(&endpoint_path, &mut endpoint, 0)
        .await?;
    agent.wait_quiet().await;
    info!("enabling the endpoint");
    let since = Instant::now();
    node.set_adm_status(&endpoint_path, &mut endpoint, 1)
        .await?;
    agent.wait_polled(since).await;

    // ---- Host: disable, enable -------------------------------------------
    info!("disabling the host");
    node.set_adm_status(&host_path, &mut host, 0).await?;
    wait_matched(syslog_port, &control, listener_item_id, false).await;
    agent.wait_quiet().await;
    info!("enabling the host");
    let since = Instant::now();
    node.set_adm_status(&host_path, &mut host, 1).await?;
    wait_matched(syslog_port, &control, listener_item_id, true).await;
    agent.wait_polled(since).await;

    // ---- Deletes ---------------------------------------------------------
    info!("deleting the listener item");
    node.delete(&format!("/items/{listener_item_id}")).await?;
    wait_matched(syslog_port, &control, listener_item_id, false).await;

    // takes the endpoint and the SNMP item with it
    info!("deleting the host");
    node.delete(&host_path).await?;
    agent.wait_quiet().await;

    // Enabled when deleted, so the node has something to stop, and with an
    // input: the delete then logs an upsert (the inputs go first) and a delete
    // in one transaction, and the node has to take the delete as the latest.
    info!("deleting the toggled sink");
    let processor_id = node
        .create(
            "/processors",
            &json!({ "name": "unused", "adm_status": 1, "config": { "type": "null" } }),
            "processor_id",
        )
        .await?;
    toggled_sink["inputs"] = json!([processor_id]);
    node.call(Method::PUT, &toggled_path, Some(&toggled_sink))
        .await?;
    node.delete(&toggled_path).await?;
    wait_sink(&node, toggled_id, false).await;
    let (token, _) = route(syslog_port, &control).await?;
    assert!(
        line_with(&toggled, &token).is_none(),
        "a deleted sink must not write"
    );

    info!("disabling, enabling and deleting the listener");
    node.set_adm_status(&listener_path, &mut listener, 0)
        .await?;
    wait_listener(&node, listener_id, syslog_port, false).await;
    node.set_adm_status(&listener_path, &mut listener, 1)
        .await?;
    wait_listener(&node, listener_id, syslog_port, true).await;
    node.delete(&listener_path).await?;
    wait_listener(&node, listener_id, syslog_port, false).await;

    node.patrolli.stop().await;
    Ok(())
}
