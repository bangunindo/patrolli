// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

// End-to-end test: a 3-node patrolli backend cluster forwards a syslog packet
// from the receiving node to the host's assigned owner.
//
// The nodes are the binary cargo built for this test, run on the host; the
// database comes from `setup::setup_db` (a Postgres container, or a database on
// `PATROLLI_TEST_DATABASE_URL`). All three nodes bind the same syslog port with
// SO_REUSEPORT, since the listener's address comes from the database. Linux
// spreads datagrams across them by 4-tuple hash, so the test sends each try
// from a fresh source port; macOS hands every one to the socket bound first,
// so the owner is restarted to be the last to bind, leaving the packets to the
// other two nodes either way.
//
// The `unforwarded_*` tests cover the rows a node keeps instead: a listener
// with `disable_cluster_forwarding`, and what the item's `unforwarded` makes
// of a row received by a node that does not monitor the item's host.

mod setup;

use anyhow::{Context, Result, bail};
use setup::patrolli::{Patrolli, free_tcp_port, free_udp_port};
use std::net::UdpSocket;
use tracing::info;
use uuid::Uuid;

const JWT_SECRET: &str = "eiK4eu4hie2oolaiph8eej2eehooNeig";

struct Node {
    name: String,
    config: String,
    prometheus_port: u16,
    patrolli: Patrolli,
}

impl Node {
    fn start(db_url: &str, node_id: Uuid, name: &str) -> Self {
        let prometheus_port = free_tcp_port();
        let inter_port = free_tcp_port();
        // Heartbeat / watcher tuned aggressively so the test doesn't wait a full
        // 5-second cycle for cluster_nodes registration and config propagation.
        let config = format!(
            r#"
global:
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
    cluster:
      node_id: {node_id}
      node_name: {name}
      heartbeat_interval: 1
      heartbeat_timeout: 3
      metadata:
        advertise_address: "127.0.0.1:{inter_port}"
      inter_node:
        grpc:
          address: "127.0.0.1:{inter_port}"
"#
        );
        let patrolli = Patrolli::start(name, &config, &[("PATROLLI_JWT_SECRET", JWT_SECRET)]);
        Self {
            name: name.to_string(),
            config,
            prometheus_port,
            patrolli,
        }
    }

    /// Stop it and start it again with the same config, and so the same node id.
    async fn restart(&mut self) {
        self.patrolli.stop().await;
        self.patrolli = Patrolli::start(
            &self.name,
            &self.config,
            &[("PATROLLI_JWT_SECRET", JWT_SECRET)],
        );
    }

    /// Every sample of the series starting with `prefix`, summed, from the
    /// Prometheus endpoint.
    async fn metric_sum(&self, prefix: &str) -> Result<f64> {
        let url = format!("http://127.0.0.1:{}/metrics", self.prometheus_port);
        let body = reqwest::get(&url)
            .await
            .and_then(|r| r.error_for_status())
            .with_context(|| format!("failed to scrape {} at {url}", self.name))?
            .text()
            .await?;
        let mut total = 0.0;
        for line in body.lines() {
            if !line.starts_with(prefix) {
                continue;
            }
            let value = line.rsplit(' ').next().unwrap_or_default();
            total += value
                .parse::<f64>()
                .with_context(|| format!("unexpected sample {line:?}"))?;
        }
        Ok(total)
    }
}

/// Starts one node per id and waits until all of them are in `cluster_nodes`.
async fn start_nodes(db_url: &str, pool: &patrolli::db::Pool, node_ids: &[Uuid]) -> Vec<Node> {
    let nodes: Vec<Node> = node_ids
        .iter()
        .enumerate()
        .map(|(i, id)| Node::start(db_url, *id, &format!("node-{}", i + 1)))
        .collect();
    info!("waiting for every patrolli node to register in cluster_nodes");
    let want = node_ids.len() as i64;
    setup::eventually(async || {
        let conn = pool.get().await?;
        let count: i64 = conn
            .query_one("SELECT count(*)::bigint FROM cluster_nodes", &[])
            .await
            .context("failed to count cluster_nodes")?
            .get(0);
        if count < want {
            bail!("{count} of {want} nodes in cluster_nodes");
        }
        Ok(())
    })
    .await;
    nodes
}

/// Waits until each of `nodes` has bound the syslog listener.
async fn all_listening(nodes: &[Node]) {
    setup::eventually(async || {
        let waiting: Vec<_> = nodes
            .iter()
            .filter(|n| !n.patrolli.stderr_contains("udp listening"))
            .map(|n| n.name.as_str())
            .collect();
        if !waiting.is_empty() {
            bail!("not listening yet: {}", waiting.join(", "));
        }
        Ok(())
    })
    .await
}

const ITEM_NAME: &str = "listener-item";

struct Seed {
    /// The node the host is pinned to.
    owner_node_id: Uuid,
    syslog_port: u16,
    disable_cluster_forwarding: bool,
    /// The item's `unforwarded`, with a second sink that has no inputs: where
    /// `passthrough` rows go. `None` leaves both out.
    unforwarded: Option<&'static str>,
}

/// A syslog listener, a host pinned to the owner (`sticky_monitored_by` keeps
/// the leader's rebalancer from moving it), its listener item matching what
/// comes from loopback, a `null` processor and a `stdout` sink behind it.
/// Returns the item's id.
async fn seed(pool: &patrolli::db::Pool, seed: Seed) -> Result<Uuid> {
    let processor_id = Uuid::now_v7();
    let syslog_listener_id = Uuid::now_v7();
    let sink_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let endpoint_id = Uuid::now_v7();
    let host_template_id = Uuid::now_v7();
    let item_id = Uuid::now_v7();

    let mut conn = pool.get().await?;
    let tx = conn.transaction().await?;
    tx.execute(
        "INSERT INTO processors (processor_id, name, type, adm_status, last_updated, config) \
         VALUES ($1, 'populate-metadata', 'null', 1, now(), 'null'::jsonb)",
        &[&processor_id],
    )
    .await
    .context("failed to insert processor")?;
    let syslog_cfg = serde_json::json!({
        "listener": { "udp": { "address": format!("0.0.0.0:{}", seed.syslog_port) } }
    });
    tx.execute(
        "INSERT INTO listeners (listener_id, name, type, adm_status, last_updated, config, \
         disable_cluster_forwarding) \
         VALUES ($1, 'syslog-1', 'syslog', 1, now(), $2, $3)",
        &[
            &syslog_listener_id,
            &syslog_cfg,
            &seed.disable_cluster_forwarding,
        ],
    )
    .await
    .context("failed to insert syslog listener")?;
    let sink_cfg = serde_json::json!({ "path": "stdout", "format": "json_otlp_denormalized" });
    tx.execute(
        "INSERT INTO sinks (sink_id, name, type, adm_status, last_updated, config) \
         VALUES ($1, 'file-stdout', 'file', 1, now(), $2)",
        &[&sink_id, &sink_cfg],
    )
    .await
    .context("failed to insert sink")?;
    tx.execute(
        "INSERT INTO sink_inputs (sink_id, input_id) VALUES ($1, $2)",
        &[&sink_id, &processor_id],
    )
    .await
    .context("failed to insert sink_inputs")?;
    if seed.unforwarded.is_some() {
        tx.execute(
            "INSERT INTO sinks (sink_id, name, type, adm_status, last_updated, config) \
             VALUES ($1, 'file-stdout-no-inputs', 'file', 1, now(), $2)",
            &[&Uuid::now_v7(), &sink_cfg],
        )
        .await
        .context("failed to insert the sink without inputs")?;
    }
    tx.execute(
        "INSERT INTO hosts (host_id, name, adm_status, last_updated, monitored_by, sticky_monitored_by) \
         VALUES ($1, 'forwarded-host', 1, now(), $2, true)",
        &[&host_id, &seed.owner_node_id],
    )
    .await
    .context("failed to insert host")?;
    let endpoint_cfg = serde_json::json!({
        // The test sends from loopback, so that is the host's source address.
        "address": "127.0.0.1:1",
        "community": "x",
    });
    tx.execute(
        "INSERT INTO host_endpoints (endpoint_id, host_id, name, type, adm_status, last_updated, config) \
         VALUES ($1, $2, 'endpoint-1', 'snmp_v2c', 1, now(), $3)",
        &[&endpoint_id, &host_id, &endpoint_cfg],
    )
    .await
    .context("failed to insert endpoint")?;
    tx.execute(
        "INSERT INTO host_templates (host_template_id, host_id) VALUES ($1, $2)",
        &[&host_template_id, &host_id],
    )
    .await
    .context("failed to insert host_template")?;
    let mut item_cfg = serde_json::json!({
        "match_strategy": { "method": "endpoint_source_ip" }
    });
    if let Some(unforwarded) = seed.unforwarded {
        item_cfg["unforwarded"] = unforwarded.into();
    }
    tx.execute(
        "INSERT INTO items (item_id, host_template_id, endpoint_id, listener_id, name, type, \
         adm_status, last_updated, config, schedule, timeout_ms) \
         VALUES ($1, $2, $3, $4, $6, 'listener', 1, now(), $5, '* * * * * * *', 10000)",
        &[
            &item_id,
            &host_template_id,
            &endpoint_id,
            &syslog_listener_id,
            &item_cfg,
            &ITEM_NAME,
        ],
    )
    .await
    .context("failed to insert item")?;
    tx.execute(
        "INSERT INTO item_processors (item_id, processor_id) VALUES ($1, $2)",
        &[&item_id, &processor_id],
    )
    .await
    .context("failed to insert item_processors")?;
    tx.commit().await?;
    Ok(item_id)
}

#[tokio::test]
async fn test_forwarder_e2e_three_node_cluster() -> Result<()> {
    setup::setup_logging();
    let (db, pool) = setup::setup_db().await;
    let syslog_port = free_udp_port();

    // ---- Three patrolli nodes ---------------------------------------------
    // The owner has to be registered before its host is seeded
    // (`hosts.monitored_by` references `cluster_nodes`), so all of them start
    // first.
    let node_ids = [Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()];
    let mut nodes = start_nodes(&db.url, &pool, &node_ids).await;

    // ---- DB seeding ----------------------------------------------------
    // node-1 is the owner of the host; sticky_monitored_by prevents the
    // leader's rebalancer from moving it.
    seed(
        &pool,
        Seed {
            owner_node_id: node_ids[0],
            syslog_port,
            disable_cluster_forwarding: false,
            unforwarded: None,
        },
    )
    .await?;

    // ---- Wait for the syslog listener on every node ---------------------
    info!("waiting for every node to bind the syslog listener");
    all_listening(&nodes).await;

    // ---- Make the owner the last to bind -------------------------------
    // The host is sticky, so it stays with node-1 across the restart.
    info!("restarting node-1");
    nodes[0].restart().await;
    all_listening(&nodes[..1]).await;

    // ---- Send syslog until the owner gets it from a peer ---------------
    // Each try comes from a fresh socket, so a new source port, which Linux
    // hashes to any of the three nodes. node-1 also writes what it receives
    // itself, so the token in its output alone proves nothing: its forwarder
    // server has to have taken records from a peer too. The forwarders on
    // node-2 and node-3 reconnect to the restarted node-1 with backoff, so the
    // first tries can fail to arrive.
    let token = format!("e2e-token-{}", Uuid::now_v7().simple());
    let syslog_msg = format!("<134>1 2026-05-18T00:00:00Z e2e-host e2e-app 1 ID47 - {token}");
    setup::eventually(async || {
        UdpSocket::bind("127.0.0.1:0")?
            .send_to(syslog_msg.as_bytes(), ("127.0.0.1", syslog_port))?;
        let received = nodes[0]
            .metric_sum("patrolli_forwarder_server_records_sent")
            .await?;
        let written = nodes[0].patrolli.stdout_contains(&token);
        if !written || received == 0.0 {
            bail!("node-1 wrote the token: {written}, took {received} records from its peers");
        }
        Ok(())
    })
    .await;

    // ---- Assertions ----------------------------------------------------
    // Sink writes json_otlp_denormalized lines; tracing logs do not contain
    // the token.
    assert!(
        !nodes[1].patrolli.stdout_contains(&token),
        "node-2 should have forwarded, not written locally"
    );
    assert!(
        !nodes[2].patrolli.stdout_contains(&token),
        "node-3 should have forwarded, not written locally"
    );

    // ---- Cleanup -------------------------------------------------------
    for node in &mut nodes {
        node.patrolli.stop().await;
    }
    Ok(())
}

/// What node-2, which does not monitor the host, did with the rows of the
/// host's item it received, on a listener with `disable_cluster_forwarding`.
struct Kept {
    /// The lines its sinks wrote that name the item (`patrolli.item.id`).
    lines: Vec<String>,
    /// Its listeners' dropped records.
    dropped: f64,
}

/// Two nodes, the host pinned to node-1, forwarding disabled on the listener
/// and `unforwarded` on its item. Sends syslog until node-2 has received some
/// of it and `done` accepts what it did with it.
async fn node_2_keeps(unforwarded: &'static str, done: impl Fn(&Kept) -> bool) -> Result<Kept> {
    setup::setup_logging();
    let (db, pool) = setup::setup_db().await;
    let syslog_port = free_udp_port();
    let node_ids = [Uuid::now_v7(), Uuid::now_v7()];
    let mut nodes = start_nodes(&db.url, &pool, &node_ids).await;
    let item_id = seed(
        &pool,
        Seed {
            owner_node_id: node_ids[0],
            syslog_port,
            disable_cluster_forwarding: true,
            unforwarded: Some(unforwarded),
        },
    )
    .await?
    .to_string();
    all_listening(&nodes).await;
    // node-2 has to get packets: on macOS the first socket bound takes them all
    nodes[0].restart().await;
    all_listening(&nodes[..1]).await;

    let token = format!("e2e-token-{}", Uuid::now_v7().simple());
    let syslog_msg = format!("<134>1 2026-05-18T00:00:00Z e2e-host e2e-app 1 ID47 - {token}");
    // A row node-2 received before its listener cache had the item matched
    // nothing, and went to the sink without inputs with no item id: only the
    // lines naming the item say what `unforwarded` did.
    let kept = setup::eventually(async || {
        UdpSocket::bind("127.0.0.1:0")?
            .send_to(syslog_msg.as_bytes(), ("127.0.0.1", syslog_port))?;
        let kept = Kept {
            lines: nodes[1]
                .patrolli
                .stdout_lines()
                .into_iter()
                .filter(|l| l.contains(&token) && l.contains(&item_id))
                .collect(),
            dropped: nodes[1]
                .metric_sum("patrolli_listener_records_dropped")
                .await?,
        };
        if !done(&kept) {
            bail!(
                "node-2 wrote {} lines of the item and dropped {} records",
                kept.lines.len(),
                kept.dropped
            );
        }
        Ok(kept)
    })
    .await;
    // node-1 never took a row from node-2
    assert_eq!(
        nodes[0]
            .metric_sum("patrolli_forwarder_server_records_sent")
            .await?,
        0.0,
        "nothing is forwarded with disable_cluster_forwarding"
    );
    for node in &mut nodes {
        node.patrolli.stop().await;
    }
    Ok(kept)
}

/// The rows ran the item's processors on node-2: its sink wrote them with the
/// resource the annotation completes (the item's name), which only a row
/// routed through the item's processors carries.
#[tokio::test]
async fn unforwarded_process_runs_the_items_processors_on_the_receiving_node() -> Result<()> {
    let kept = node_2_keeps("process", |kept| !kept.lines.is_empty()).await?;
    assert!(
        kept.lines.iter().all(|l| l.contains(ITEM_NAME)),
        "every row of the item went through its processors: {:?}",
        kept.lines
    );
    assert_eq!(kept.dropped, 0.0);
    Ok(())
}

/// The rows skipped the processors and reached the sink without inputs,
/// naming only their item.
#[tokio::test]
async fn unforwarded_passthrough_reaches_the_sinks_without_inputs() -> Result<()> {
    let kept = node_2_keeps("passthrough", |kept| !kept.lines.is_empty()).await?;
    assert!(
        !kept.lines.iter().any(|l| l.contains(ITEM_NAME)),
        "no row of the item went through its processors: {:?}",
        kept.lines
    );
    assert_eq!(kept.dropped, 0.0);
    Ok(())
}

/// The rows reached no sink, and the listener counted them.
#[tokio::test]
async fn unforwarded_drop_writes_nothing() -> Result<()> {
    let kept = node_2_keeps("drop", |kept| kept.dropped > 0.0).await?;
    assert!(
        kept.lines.is_empty(),
        "no row of the item reached a sink: {:?}",
        kept.lines
    );
    Ok(())
}
