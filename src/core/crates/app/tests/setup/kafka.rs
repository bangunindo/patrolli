// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Disposable, host-accessible Kafka-protocol brokers for the Kafka sink's tests and its
//! throughput benchmark: Apache Kafka, and Redpanda as the most common drop-in alternative.

#![allow(dead_code)]

use anyhow::{Context, Result, bail, ensure};
use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
use rdkafka::client::DefaultClientContext;
use rdkafka::consumer::{BaseConsumer, Consumer};
use rdkafka::message::Message;
use rdkafka::{ClientConfig, Offset, TopicPartitionList};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};
use testcontainers::core::{CmdWaitFor, ContainerPort, ExecCommand, Mount, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

// A path, not `super::upstream`: `bench_kafka` includes this file without the rest of `setup`.
#[path = "upstream.rs"]
#[allow(clippy::duplicate_mod)]
mod upstream;

/// Pinned, like the ClickHouse fixture; `PATROLLI_TEST_UPSTREAM=latest` runs against `latest`
/// instead, so a new broker release that breaks the sink still fails these tests before a
/// deployment finds it.
const APACHE_IMAGE: (&str, &str) = ("apache/kafka", "4.3.1");
const REDPANDA_IMAGE: (&str, &str) = ("redpandadata/redpanda", "v26.2.3");
/// The apache/kafka image's default `log.dirs`.
const KAFKA_LOG_DIR: &str = "/tmp/kraft-combined-logs";
const READY_TIMEOUT: Duration = Duration::from_secs(90);

/// Which broker implementation to run. Both speak the Kafka protocol; the point of running
/// the same test against each is to catch behaviour librdkafka papers over on one only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Apache,
    Redpanda,
}

impl Flavor {
    /// The image this flavor runs, as `(name, tag)`.
    fn image_ref(self) -> (&'static str, &'static str) {
        let (name, pinned) = match self {
            Flavor::Apache => APACHE_IMAGE,
            Flavor::Redpanda => REDPANDA_IMAGE,
        };
        (name, upstream::upstream_tag(pinned))
    }

    fn image(self, bootstrap: &str, port: u16) -> testcontainers::ContainerRequest<GenericImage> {
        let (name, tag) = self.image_ref();
        let request = match self {
            Flavor::Apache => GenericImage::new(name, tag)
                .with_wait_for(WaitFor::Nothing)
                .with_mapped_port(port, ContainerPort::Tcp(9092))
                .with_startup_timeout(READY_TIMEOUT)
                .with_env_var("KAFKA_NODE_ID", "1")
                .with_env_var("KAFKA_PROCESS_ROLES", "broker,controller")
                .with_env_var(
                    "KAFKA_LISTENERS",
                    "HOST://:9092,INTERNAL://:19092,CONTROLLER://:29093",
                )
                .with_env_var(
                    "KAFKA_ADVERTISED_LISTENERS",
                    format!("HOST://{bootstrap},INTERNAL://localhost:19092"),
                )
                .with_env_var(
                    "KAFKA_LISTENER_SECURITY_PROTOCOL_MAP",
                    "HOST:PLAINTEXT,INTERNAL:PLAINTEXT,CONTROLLER:PLAINTEXT",
                )
                .with_env_var("KAFKA_INTER_BROKER_LISTENER_NAME", "INTERNAL")
                .with_env_var("KAFKA_CONTROLLER_LISTENER_NAMES", "CONTROLLER")
                .with_env_var("KAFKA_CONTROLLER_QUORUM_VOTERS", "1@localhost:29093")
                .with_env_var("CLUSTER_ID", "4L6g3nShT-eMCtK--X86sw")
                .with_env_var("KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR", "1")
                .with_env_var("KAFKA_TRANSACTION_STATE_LOG_REPLICATION_FACTOR", "1")
                .with_env_var("KAFKA_TRANSACTION_STATE_LOG_MIN_ISR", "1")
                .with_env_var("KAFKA_AUTO_CREATE_TOPICS_ENABLE", "false")
                // Whole-topic removal uses the broker default delay, even
                // when a topic overrides file.delete.delay.ms.
                .with_env_var("KAFKA_LOG_SEGMENT_DELETE_DELAY_MS", "1000")
                .with_env_var("KAFKA_GROUP_INITIAL_REBALANCE_DELAY_MS", "0")
                .with_env_var("KAFKA_HEAP_OPTS", "-Xms512m -Xmx1g"),
            // `dev-container` mode is Redpanda's documented single-node test setup. It turns
            // topic auto-creation *on*, which `Broker::start_flavor` switches back off once the
            // broker is up, so a missing topic fails the same way on both flavors.
            //
            // Seastar reserves its AIO events from the host-wide `fs.aio-max-nr` (65536 by
            // default), about 11k per broker of which 10k are networking control blocks, so the
            // seventh broker a test binary starts in parallel refuses to boot. 1000 blocks
            // brings that to about 2k, plenty for a test broker.
            Flavor::Redpanda => GenericImage::new(name, tag)
                .with_wait_for(WaitFor::Nothing)
                .with_mapped_port(port, ContainerPort::Tcp(9092))
                .with_startup_timeout(READY_TIMEOUT)
                .with_cmd([
                    "redpanda".to_owned(),
                    "start".to_owned(),
                    "--mode".to_owned(),
                    "dev-container".to_owned(),
                    "--smp".to_owned(),
                    "1".to_owned(),
                    "--memory".to_owned(),
                    "1G".to_owned(),
                    "--max-networking-io-control-blocks".to_owned(),
                    "1000".to_owned(),
                    "--node-id".to_owned(),
                    "0".to_owned(),
                    "--kafka-addr".to_owned(),
                    "PLAINTEXT://0.0.0.0:9092".to_owned(),
                    "--advertise-kafka-addr".to_owned(),
                    format!("PLAINTEXT://{bootstrap}"),
                ]),
        };
        // `KAFKA_BENCH_TMPFS=1`: keep the log in memory, so the benchmark
        // measures the client rather than the disk.
        if self == Flavor::Apache && std::env::var("KAFKA_BENCH_TMPFS").as_deref() == Ok("1") {
            request.with_mount(Mount::tmpfs_mount(KAFKA_LOG_DIR))
        } else {
            request
        }
    }

    fn description(self) -> String {
        let (name, tag) = self.image_ref();
        format!("{name}:{tag}")
    }
}

/// One message read back from a topic.
#[derive(Debug, Clone)]
pub struct Consumed {
    pub partition: i32,
    pub key: Option<Vec<u8>>,
    pub payload: Vec<u8>,
}

pub struct Broker {
    pub bootstrap: String,
    pub description: String,
    admin: AdminClient<DefaultClientContext>,
    probe: Arc<BaseConsumer>,
    // Drop the clients before stopping Kafka to avoid spurious teardown errors.
    _container: Option<ContainerAsync<GenericImage>>,
}

impl Broker {
    /// The benchmark's broker: `KAFKA_BENCH_BOOTSTRAP_SERVERS` if set, else a disposable Apache
    /// Kafka.
    pub async fn start() -> Result<Self> {
        match std::env::var("KAFKA_BENCH_BOOTSTRAP_SERVERS") {
            Ok(bootstrap) => {
                ensure!(!bootstrap.trim().is_empty(), "empty bootstrap servers");
                Self::connect(bootstrap, None, "external plaintext broker".to_owned()).await
            }
            Err(_) => Self::start_flavor(Flavor::Apache).await,
        }
    }

    /// A disposable broker of the given flavor, with topic auto-creation off.
    pub async fn start_flavor(flavor: Flavor) -> Result<Self> {
        let mut started = None;
        for attempt in 1..=3 {
            // Docker must advertise the port the host client can reach. The
            // probe is released before Docker binds; retry only bind races.
            let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
            let bootstrap = format!("127.0.0.1:{port}");
            match flavor.image(&bootstrap, port).start().await {
                Ok(container) => {
                    started = Some((bootstrap, container));
                    break;
                }
                Err(e) => {
                    let reason = e.to_string();
                    if attempt == 3
                        || !(reason.contains("allocated")
                            || reason.contains("address already in use"))
                    {
                        return Err(e).with_context(|| {
                            format!("starting {flavor:?}; a local Docker daemon is required")
                        });
                    }
                }
            }
        }
        let (bootstrap, container) = started.context("could not allocate broker port")?;
        let broker = Self::connect(bootstrap, Some(container), flavor.description()).await?;
        if flavor == Flavor::Redpanda {
            broker
                .exec(&[
                    "rpk",
                    "cluster",
                    "config",
                    "set",
                    "auto_create_topics_enabled",
                    "false",
                ])
                .await?;
        }
        Ok(broker)
    }

    async fn connect(
        bootstrap: String,
        container: Option<ContainerAsync<GenericImage>>,
        description: String,
    ) -> Result<Self> {
        let mut config = ClientConfig::new();
        config
            .set("bootstrap.servers", &bootstrap)
            .set("socket.timeout.ms", "2000");
        let admin = config.create()?;
        config.set(
            "group.id",
            format!("patrolli-test-probe-{}", uuid::Uuid::now_v7()),
        );
        config.set("enable.auto.commit", "false");
        let broker = Self {
            bootstrap,
            description,
            _container: container,
            admin,
            probe: Arc::new(config.create()?),
        };
        if let Err(error) = broker.wait_ready(None, 0).await {
            broker.print_container_logs().await;
            return Err(error);
        }
        Ok(broker)
    }

    /// Run a command inside the broker's container and require it to succeed.
    async fn exec(&self, cmd: &[&str]) -> Result<()> {
        let container = self
            ._container
            .as_ref()
            .context("no container to exec in")?;
        let mut result = container
            .exec(
                ExecCommand::new(cmd.iter().copied()).with_cmd_ready_condition(CmdWaitFor::exit()),
            )
            .await
            .with_context(|| format!("exec {cmd:?}"))?;
        let code = result.exit_code().await?;
        if code != Some(0) {
            let stderr = result.stderr_to_vec().await.unwrap_or_default();
            bail!(
                "{cmd:?} exited with {code:?}: {}",
                String::from_utf8_lossy(&stderr)
            );
        }
        Ok(())
    }

    async fn print_container_logs(&self) {
        if let Some(container) = &self._container {
            let logs = async {
                for (stream, output) in [
                    ("stdout", container.stdout_to_vec().await),
                    ("stderr", container.stderr_to_vec().await),
                ] {
                    if let Ok(bytes) = output {
                        let tail = &bytes[bytes.len().saturating_sub(8192)..];
                        eprintln!(
                            "broker startup {stream}:\n{}",
                            String::from_utf8_lossy(tail)
                        );
                    }
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(5), logs).await;
        }
    }

    async fn wait_ready(&self, topic: Option<&str>, partitions: i32) -> Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if let Some(container) = &self._container {
                ensure!(
                    container.is_running().await?,
                    "Kafka container exited before becoming ready"
                );
            }
            let probe = self.probe.clone();
            let topic = topic.map(str::to_owned);
            let result = tokio::task::spawn_blocking(move || {
                let metadata = probe.fetch_metadata(topic.as_deref(), Duration::from_secs(2))?;
                ensure!(!metadata.brokers().is_empty(), "no brokers in metadata");
                if let Some(topic) = topic {
                    let entry = metadata
                        .topics()
                        .iter()
                        .find(|t| t.name() == topic)
                        .context("topic not visible")?;
                    ensure!(
                        entry.error().is_none(),
                        "topic metadata error: {:?}",
                        entry.error()
                    );
                    ensure!(
                        entry.partitions().len() == partitions as usize,
                        "partition count mismatch"
                    );
                    ensure!(
                        entry
                            .partitions()
                            .iter()
                            .all(|p| p.leader() >= 0 && p.error().is_none()),
                        "partition leader unavailable"
                    );
                    // Metadata names a leader once the controller elects it, which can be
                    // before that broker serves the partition: until then ListOffsets (and so
                    // `consume_all`'s watermarks) answers NotLeaderForPartition.
                    for partition in entry.partitions() {
                        probe
                            .fetch_watermarks(&topic, partition.id(), Duration::from_secs(2))
                            .with_context(|| format!("partition {} not served", partition.id()))?;
                    }
                }
                Ok::<_, anyhow::Error>(())
            })
            .await?;
            match result {
                Ok(()) => return Ok(()),
                Err(e) if Instant::now() >= deadline => {
                    return Err(e).context("Kafka metadata readiness timed out after 90 seconds");
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
    }

    /// Stops the broker's container, so clients see the whole cluster go away. Its data and
    /// its host port stay with the container for [`Self::resume`].
    pub async fn stop(&self) -> Result<()> {
        let container = self
            ._container
            .as_ref()
            .context("not a disposable broker")?;
        container.stop_with_timeout(Some(0)).await?;
        Ok(())
    }

    /// Starts a broker [`Self::stop`]ped, and waits until `topic` has its leaders back.
    pub async fn resume(&self, topic: &str, partitions: i32) -> Result<()> {
        let container = self
            ._container
            .as_ref()
            .context("not a disposable broker")?;
        container.start().await?;
        self.wait_ready(Some(topic), partitions).await
    }

    pub async fn create_topic(&self, name: &str, partitions: i32) -> Result<()> {
        let topic = NewTopic::new(name, partitions, TopicReplication::Fixed(1))
            .set("cleanup.policy", "delete")
            .set("retention.ms", "3600000")
            // High-throughput cases write many GiB. Reclaim each deleted topic
            // promptly instead of retaining its segments for the default minute.
            .set("file.delete.delay.ms", "1000");
        let opts = AdminOptions::new().request_timeout(Some(Duration::from_secs(30)));
        for result in self.admin.create_topics(&[topic], &opts).await? {
            result.map_err(|e| anyhow::anyhow!("create topic: {e:?}"))?;
        }
        self.wait_ready(Some(name), partitions).await
    }

    pub async fn verify_count(&self, name: &str, partitions: i32, expected: u64) -> Result<()> {
        let probe = self.probe.clone();
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let mut count = 0;
                for partition in 0..partitions {
                    let left = deadline.saturating_duration_since(Instant::now());
                    ensure!(!left.is_zero(), "offset verification deadline exceeded");
                    let (low, high) = probe.fetch_watermarks(
                        &name,
                        partition,
                        left.min(Duration::from_secs(2)),
                    )?;
                    ensure!(
                        low == 0,
                        "topic lost records to retention: low offset {low}"
                    );
                    count += high as u64;
                }
                if count == expected {
                    return Ok(());
                }
                if count > expected || Instant::now() >= deadline {
                    bail!("broker offsets contain {count} messages, expected {expected}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .await?
    }

    /// Messages the topic holds once its offsets reach `expected` or stop moving (for a
    /// producer whose losses go unreported, as with `acks=0`).
    pub async fn settled_count(&self, name: &str, partitions: i32, expected: u64) -> Result<u64> {
        let probe = self.probe.clone();
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut last = None;
            loop {
                let mut count = 0;
                for partition in 0..partitions {
                    let (_, high) =
                        probe.fetch_watermarks(&name, partition, Duration::from_secs(2))?;
                    count += high as u64;
                }
                if count >= expected || last == Some(count) || Instant::now() >= deadline {
                    return Ok(count);
                }
                last = Some(count);
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .await?
    }

    pub async fn delete_topic(&self, name: &str) -> Result<()> {
        let opts = AdminOptions::new().request_timeout(Some(Duration::from_secs(30)));
        for result in self.admin.delete_topics(&[name], &opts).await? {
            result.map_err(|e| anyhow::anyhow!("delete topic: {e:?}"))?;
        }
        Ok(())
    }

    /// Every message in `name`, read from the start of each partition up to the high watermark
    /// taken first. Call it after the producer has flushed, so the watermark covers everything
    /// the test produced; with `acks: all` the watermark is final once the flush returns.
    pub async fn consume_all(&self, name: &str) -> Result<Vec<Consumed>> {
        let bootstrap = self.bootstrap.clone();
        let probe = self.probe.clone();
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || {
            let metadata = probe.fetch_metadata(Some(&name), Duration::from_secs(5))?;
            let topic = metadata
                .topics()
                .iter()
                .find(|t| t.name() == name)
                .context("topic not visible")?;
            ensure!(
                topic.error().is_none(),
                "topic metadata error: {:?}",
                topic.error()
            );
            let mut assignment = TopicPartitionList::new();
            let mut expected = 0usize;
            for partition in topic.partitions() {
                let (low, high) =
                    probe.fetch_watermarks(&name, partition.id(), Duration::from_secs(5))?;
                expected += (high - low) as usize;
                assignment.add_partition_offset(&name, partition.id(), Offset::Beginning)?;
            }
            // Assigned, not subscribed: no group join, no rebalance to wait out.
            let consumer: BaseConsumer = ClientConfig::new()
                .set("bootstrap.servers", &bootstrap)
                .set(
                    "group.id",
                    format!("patrolli-test-reader-{}", uuid::Uuid::now_v7()),
                )
                .set("enable.auto.commit", "false")
                .create()?;
            consumer.assign(&assignment)?;
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut out = Vec::with_capacity(expected);
            while out.len() < expected {
                ensure!(
                    Instant::now() < deadline,
                    "read only {} of {expected} messages from {name}",
                    out.len()
                );
                match consumer.poll(Duration::from_millis(500)) {
                    None => {}
                    Some(Err(e)) => return Err(e).context("consume"),
                    Some(Ok(m)) => out.push(Consumed {
                        partition: m.partition(),
                        key: m.key().map(<[u8]>::to_vec),
                        payload: m.payload().unwrap_or_default().to_vec(),
                    }),
                }
            }
            Ok(out)
        })
        .await?
    }
}
