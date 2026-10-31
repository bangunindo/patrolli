# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`patrolli` — the Rust core of the Patrolli monitoring / telemetry platform (crate `patrolli`, edition 2024).
It lives at `src/core` inside a monorepo; the sibling `src/web` is the Nuxt/Vue frontend, which
generates its API client from the `openapi.json` produced by this crate. All commands below assume
`src/core` as the working directory.

Two binaries: `patrolli` (`src/main.rs`, the daemon) and `schema_gen` (`src/schema_gen.rs`, writes
`openapi.json`). `src/lib.rs` declares every module (`pub`, so integration tests can drive
internals); both binaries reach them through `use patrolli::…`. Never add a `mod` for a lib module
to `main.rs`: the binary would compile its own copy of the whole crate, doubling every rebuild and
running each unit test twice. Unit tests live in the lib (`cargo test --lib`).

Telemetry is carried end to end as OpenTelemetry Arrow record batches (`payload::Payload` wraps
`OtapArrowRecords` from `otel-arrow-dfe-pdata`), never as row-oriented structs. Anything that
touches signal data works on Arrow columns.

## Commands

```bash
cargo build                                   # debug (line-tables-only debuginfo, see [profile.dev])
cargo build --release --bin patrolli
cargo build --profile profile                 # release + debuginfo, for jeprof/perf
cargo run --bin patrolli -- --config config-backend.yaml
cargo fmt --all
cargo clippy --all-targets
cargo run --bin schema_gen                    # regenerates ./openapi.json (writes relative to CWD)
cargo bench --bench bench_channels            # criterion
```

Native build deps (see `Dockerfile`): `protobuf-compiler` (`build.rs` compiles the cluster proto),
`cmake` + `libsasl2-dev` (rdkafka), `libcurl4-openssl-dev` (headers only: librdkafka 2.12's
`rdkafka_conf.c` includes `<curl/curl.h>` under an `#ifdef` that is always true, though curl is
off), `libclang-dev`, `libunwind-dev` (jemalloc profiling).

### Tests

```bash
cargo test --lib                              # unit tests only — no Docker needed
cargo test --test cluster_config_watcher      # one integration test binary
cargo test --lib cache::lookup_table::         # filter unit tests by module path
cargo test --test otel_e2e_test -- --nocapture
```

No test builds a patrolli image: the end-to-end tests (`otel_e2e_test`, `cluster_forwarder_e2e`,
`config_propagation_e2e`, like `shutdown_test`) run the `patrolli` binary cargo builds for them
(`CARGO_BIN_EXE_patrolli`), through `tests/setup::patrolli::Patrolli`. Docker is still needed for third-party
servers: `clickhouse_insert_test` and `kafka_sink_test` (their servers), `otel_e2e_test` (OTel
collectors and telemetrygen, which reach patrolli on the host through the Docker host gateway, so a host
firewall must let container → host traffic through), and the `cluster_*` and
`config_propagation_e2e` binaries only for Postgres, which `PATROLLI_TEST_DATABASE_URL` replaces.
`shutdown_test` needs none. `tests/setup::setup_db` gives each test its own migrated Postgres
database: a `postgres:18-alpine` container by default, or, with `PATROLLI_TEST_DATABASE_URL` set to a
server the user can `CREATE DATABASE` on, a fresh `patrolli_test_<uuid>` database there, dropped when
the test's `TestDb` guard is.

`tests/setup::setup_logging` writes through libtest's capture: a passing test prints nothing, a
failing one shows its own logs, `--nocapture` shows everything. `RUST_LOG` sets the filter in
`tracing_subscriber::filter::Targets` syntax (`RUST_LOG=info,patrolli=trace`), default `info`.
Third-party images are pinned to exact versions; the Kafka, Redpanda and ClickHouse servers
the sink tests run against switch to `latest` with `PATROLLI_TEST_UPSTREAM=latest`, for catching an
upstream release that breaks a sink. Wait for a condition with `setup::eventually`, not a sleep.

`tests/bench_*.rs` are throughput harnesses, not assertions (only `bench_kafka` wants Docker, for a
disposable broker). Their harness test is
`#[ignore]`d, so a bare `cargo test` only compiles them (it still runs the few small tests next to
it); run one in release with `cargo test --release --test <name> -- --ignored --nocapture`.

## Two run modes

`main.rs` branches on whether `global.backend` is present in the YAML:

- **local mode** (`local_run`): the whole config — listeners, hosts, endpoints, items, processors,
  sinks, lookup tables — comes from the YAML file and is static for the process lifetime.
- **backend mode** (`backend_run`): only `global` is read from disk. Everything else lives in
  PostgreSQL, is served by the axum API, and is hot-reloaded into the in-memory caches by
  `cluster::config_watcher`. This mode also starts the cluster machinery and the API server.

`config-full.yaml` is the annotated reference for every recognized option; `config-backend.yaml` is a
runnable starting point for backend mode.

Startup order in `main` matters: logging → MIB (`common::mib::init`) → `config.validate()` → disk
channel database. OID validation resolves names against the MIB, so validation cannot run first.

## Data flow

```
listener::WorkerGroup  (syslog, netflow, snmp_trap, otel, dummy,  ─┐
                        internal_logs, internal_metrics)            │  chan (topic "processor")
monitor::WorkerGroup   (snmp, snmp_walk, prometheus_scrape,         │
                        dummy — scheduler-driven)                   ┘        │
                                                                             ▼
                                            processor::WorkerGroup — one OS thread per core,
                                            each with its own tokio LocalRuntime, sync hot path
                                                                             │  chan (topic = sink_id)
                                                                             ▼
                                            sink::WorkerGroup  (file, kafka, otel/OTAP, clickhouse)
```

- **`chan`** is the queue between every stage: `chan::new` hands back an `AdminChannel` (owns topics
  and metrics) and a `SenderChannel`. A topic is either memory-backed (flume) or disk-backed (fjall,
  each payload as plain Arrow IPC streams, optionally transport-optimized and compressed —
  `chan::codec`) per `config::Channel`; the fjall database is a process-global initialized once
  from `global.disk_channel.data_dir`.
- The `netflow` listener also handles sFlow on the same socket — `sflow::is_sflow` sniffs each
  datagram, so there is no separate sFlow listener type.
- **UDP intake** for syslog and netflow is `common::udp_listener`: one `SO_REUSEPORT` socket per
  socket-level worker, each read by a dedicated OS thread (`udp-recv-N`, `recvmmsg`), never a tokio
  task, so parsing load cannot starve the kernel receive queue. `tests/bench_udp_listener.rs` is its
  Docker-free throughput/`RcvbufErrors` harness (`cargo test --release --test bench_udp_listener -- --ignored --nocapture`).
- **Listener annotation** (`listener::annotate`) is where an inbound packet is matched to an item /
  host / processor by source IP or attribute match, entirely with Arrow compute kernels. Its output
  splits into `for_process` (this node) and `for_forward` (records owned by another cluster node).
- **The pipeline is a DAG**, not a chain. `cache::processor` builds a `petgraph` acyclic graph of
  `Node::Processor` / `Node::Sink` and compiles it into `Step` trees published through an `ArcSwap`,
  so processor workers read routes lock-free. `Processor::compile` must be called after mutating the
  graph. Processors themselves are the small set in `config::processor::Config` (null, attribute
  filter/add/drop/rename, `attribute_lookup`, which enriches attributes from `cache::lookup_table`,
  and `transform`, a program (OPL for now, keyed by language) on otel-arrow's query engine — each processor worker keeps its own
  lazily planned engine per processor, see `processor::transform`).
- **Caches** (`cache::*`) are the hot, in-memory view of configuration that the data path reads:
  `item`, `endpoint`, `listener`, `host`, `processor`, `lookup_table`. `cache::get_cache()` returns
  the process-wide singletons (each module holds a `LazyLock`); `Cache::new()` builds an isolated one
  for tests. Lookup tables are read synchronously because the processor hot path is a sync fn.

## Cluster (backend mode)

- `cluster::node_lock` arbitrates roles via Postgres advisory locks (`db::AdvisoryLock`). A node is
  `Leader`, `Main`, `Standby`, or `LostConnection` (`NodeStateType`); `is_main()` gates whether the
  node runs workers.
- Host placement is decided by the leader each tick: `NodeLock::leader_tasks` gathers per-node
  loads and applies what the pure `cluster::balancer::plan` returns. `configs.cluster`
  (`ClusterPolicy`, re-read every tick, last good value kept on a bad edit) caps how many live
  nodes carry hosts with `max_active_nodes` (`0` = all); the other live nodes stay empty as spares.
  A node stale past `heartbeat_interval + 2s` (or `heartbeat_timeout`, if shorter) hands its hosts
  as one block to a spare, a returning node becomes the spare (nothing moves back), and lowering
  the cap evacuates one node per tick. Hosts always move straight to their new owner, never via
  `monitored_by = NULL` (each change makes the hosts trigger log to every node), and an expired
  node is deleted only once it owns no hosts. Sticky hosts are invisible to the balancer and never
  moved: a dead node still owning one keeps its `cluster_nodes` row (inactive, logs purged every
  tick) until it or its standby returns.
- `cluster::config_watcher` polls the database (`watcher_interval`) and diffs it into the caches and
  worker groups, adding/removing listeners, monitors and sinks in place. It aborts with
  `NodeLockLost` if the node's lock goes away mid-transaction.
  It also re-reads `configs.monitor` every poll (one row, compared by value, no cluster log) and
  applies it with `monitor::WorkerGroup::apply_config`: workers via `adjust`, and the batch through
  a `watch` the batcher follows, flushing what it holds. A bad edit keeps the last good value.
- `cluster::inter` is a private patrolli-to-patrolli gRPC service (`inter_cluster.proto`, compiled by
  `build.rs`, exposed as `crate::cluster::inter::v1`). It carries the forwarder (records for hosts
  owned by a peer), item stats, and processor capture streams. Not an OpenTelemetry endpoint.

## API layer

axum + utoipa. Routes are declared in `api::create_router`; `/api/v1/**` and `/ws/v1/**` sit behind
`api::jwt::require_auth`, except `/api/v1/auth/{login,logout,refresh}` and `/health`. Swagger UI at `/swagger-ui`, spec at
`/api-docs/openapi.json`.

Convention: one file per endpoint under `src/api/handlers/<resource>/<verb_resource>.rs`, re-exported
from that resource's `mod.rs`, which also holds the resource's `#[derive(OpenApi)]` handler struct
that `ApiDoc` nests. Responses always go through `api::response::{ResponseData, ResponseError}`.
SQL is built with `sea-query` against the column enums in `db::schema` — no raw string SQL in
handlers.

`ApiDoc` takes its version from `Cargo.toml` (utoipa's default for an omitted `info(version)`), but
`openapi.json` is committed: rerun `schema_gen` after a version bump.

## Conventions that repeat everywhere

- **Internal metrics**: each subsystem keeps an `InternalMetrics` struct of `Atomic*` counters, a
  `get()` that snapshots it into a `common::internal_metrics::*` type, and a `spawn_metrics_reporter`
  task that stores the snapshot into the global `INTERNAL_METRICS` (`ArcSwap`) on
  `global.internal_metrics_interval`. The `internal_metrics` listener (one report per node, told
  apart by the scope's `patrolli.node.id`) and the Prometheus endpoint both read from there. Follow
  this shape when adding a subsystem.
- **Config validation**: every `config::*` type implements `validate(&mut self)` that normalizes
  defaults (e.g. `worker_count: 0` → `available_parallelism()`) and is called top-down from
  `LocalFileConfig::validate`.
- **Config parsing is strict**: every `config::*` type is `#[serde(deny_unknown_fields)]`, so a
  misplaced key fails instead of being dropped. A struct that `flatten`s an *internally* tagged
  enum can't carry it (the enum borrows every key, so the parent rejects all of them): put it on
  the variants instead, as `ProcessorConfig` does with `MemoryChannel`/`DiskChannel`. The only
  other exceptions are `PartialConfig` (peeks at `global` in any file) and `NodeMetadata` (read
  across versions). `Batch` sits on the listener/sink parent next to `type`, never under `config:`.
  `config::tests::the_shipped_configs_deserialize` loads `config-full.yaml`, so keep it
  current when a field moves.
- **Worker supervision**: listener and sink `WorkerGroup`s run a supervisor (`spawn_supervisor`)
  that respawns any unit whose `listen()` / `run()` returned without being cancelled, with
  exponential backoff (`common::backoff`). So return only on cancel, on your input ending, or on a
  failure the worker cannot get past, and keep `total_workers` balanced on every return.
- **Listener transports** (syslog, netflow, otel) belong to the listener, not to its workers.
  Workers start one lazily through `ensure_started` (so a failed bind retries with the
  supervisor's backoff) and never stop it: it survives scale-downs and worker restarts with its
  sockets and connections. Its state is `Idle | Running | Closed`. Only producers (reader threads,
  servers, streams) hold the `Sender` of the workers' channel, so the channel disconnects exactly
  when they are done. A producer that dies cancels the intake token (`drop_guard`), the workers
  see the disconnect and return, and the supervisor's restart rebuilds the transport.
- **Shutdown**: `CancellationToken` throughout, plus explicit ordered teardown in `local_run` /
  `Cluster::run` — drain producers before consumers so nothing is lost. The same holds inside a
  listener: `Worker::close` calls `Listener::stop` (intake off, producers hand over to the
  still-running workers and drop their senders, state becomes `Closed`), then cancels the workers,
  which drain what is queued once the channel is disconnected. One `DRAIN_TIMEOUT` bounds the whole
  close; past it the workers are aborted, and a `Running` dropped mid-`stop` aborts its tasks
  (they live in `JoinSet`s). Don't thread extra abort tokens through a transport for this.
  Processors and sinks end the same way: on their input disconnecting, not on a token. Shutdown
  closes the sink topics with `AdminChannel::close_all` (`remove_all` would delete disk data), so
  sink workers deliver what is queued; a config change instead hands the topic to the replacing
  sink (`Input::HandedOver`) and cancels the old workers. A *channel* change swaps the topic itself
  (`AdminChannel::replace`) and moves what the old consumer holds over with `Consumer::adopt`. `sink::Worker::close` bounds each phase
  (workers, then `Sink::close`) by its own `DRAIN_TIMEOUT`. Workers still draining an ending input
  then are cancelled and get `CANCEL_GRACE` to nack what they hold; whatever still runs is aborted.
  A sink whose destination is down never drains, and the processors closed before it would block
  on its full topic forever, so `processor::WorkerGroup::close` waits only its own `DRAIN_TIMEOUT`,
  then cancels `stop_waiting`: `SenderChannel::send_to_until` turns them away from a full memory
  topic and writes past `max_disk_size` on a disk one. Any send a teardown waits on must be raced
  against a token in the same way (the monitor worker → batcher → processor channel sends are).
  Every stage's timeout is cut by one `common::shutdown::Deadline`, set by
  `global.shutdown_timeout` from the signal (`Deadline::NONE` outside shutdown); only the
  hand-over graces (`CANCEL_GRACE`, the processors' `STOP_GRACE`) ignore it. `setup_shutdown` in
  `main.rs` exits the process on a second signal (130/143) or 5s past the deadline (1), flushing
  the log writer first (`force_exit`). `tests/shutdown_test.rs` covers this without Docker.
- **Panics are fatal**: `common::panic::install` (right after `init_logging`) sets a hook that
  logs the panic through tracing (`panicked, shutting down`, with `location`), cancels the root
  shutdown token, and the process exits with 70 after the orderly teardown, for the service
  manager to restart it. No respawn: `parking_lot` locks do not poison, so a respawned worker
  could read what the panic left half written. Release unwinds (not `panic = 'abort'`) so the
  teardown can run. So supervisors restart only units that *return*, and a teardown never
  re-raises a `JoinError`: await its steps with `common::shutdown::join_step`, never `?` or
  `JoinSet::join_all`. Don't `panic!` on a recoverable failure (a bind, a closed peer): return
  it, and cancel the token if the node cannot go on.
- **Sink retries**: every sink is a `sink::export::ExportSink` (ClickHouse, Kafka, file, OTAP)
  over an `Exporter` (`encode` once, `export` per attempt, answering `Delivered`/`Retry`/`Reject`/`Partial`)
  and the driver owns batching, the sink's `retry:` backoff, metrics and ack/nack. A worker never
  returns because its destination failed; it retries the batch it holds, and nacks it only when
  cancelled. Classify an error as `Retry` only when sure it is transient.
- **Types**: `SmolStr` for short strings, `FxHashMap`/`ahash` over std hashing, `parking_lot` locks
  in sync code, `papaya`/`ArcSwap` where readers must not block. IDs are UUIDv7 (`Uuid::now_v7`).
- **Migrations**: refinery, embedded from `migrations/` as `V<n>__<name>.sql`; they run
  automatically on backend startup under an advisory lock. Add a new file, never edit an applied one,
  and mirror any schema change in `db::schema`.
- **Logging**: see the dedicated section below — it is a convention, not a preference.

## Logging

`tracing`, with a `TracingRateLimitLayer` installed in `init_logging` (`src/main.rs`). A throttle
signature is `(level, target, message text, field values)` with `error` and `request_id` excluded,
so **an interpolated message defeats the throttle**: every distinct error string becomes a fresh
signature that is never suppressed and evicts the 10k-signature LRU. That is the mechanical reason
behind rules 2 and 3, not a style opinion.

### Levels

| Level | Meaning | Volume at steady state |
|---|---|---|
| `error` | operator must act; invariant broken or node degraded | rare, alertable |
| `warn` | degraded but self-healing, or data deliberately dropped | bounded |
| `info` | lifecycle state changes only (start/stop/bind/config applied) | near-constant |
| `debug` | per config object, per connection, per batch | scales with config |
| `trace` | per record, per packet | scales with traffic |

A caller's or a peer's bad input is never `error` — that is a response status or a `warn`. A remote
endpoint the client reconnects to is `warn`, not `error`; only misconfiguration and broken
invariants are `error`.

### Rules

1. **Log an error exactly once, where it is swallowed.** A function returning `Result` does not log;
   the caller that decides to degrade or drop does. Logging *and* returning the same error is the
   most common mistake here.
2. **The message is a constant literal** — lowercase, no trailing period, verb phrase, no
   interpolation. One-shot startup banners in `main.rs` (the heap-profiling instructions) are the
   deliberate exception: they are prose printed once, not events to aggregate.
3. **Values go in fields, never the message**: `error!(error = %e, addr = %addr, "bind failed")`.
4. **One spelling for errors**: `error = %e` when `Display` is complete, `error = %format!("{e:#}")`
   for `anyhow` (plain `{}` prints only the outermost context and drops the chain). `?` is only for
   values with no `Display` impl at all.
5. **Identity goes on the span, not the event.** `<entity>_id` for the UUID and `<entity>` for the
   name — `sink_id`/`sink`, `item_id`/`item`, `listener_id`/`listener`, `node_id`/`node`,
   `processor_id`, `host_id`/`host`. Set both once on the task's
   `#[tracing::instrument(skip_all, fields(...))]`; events inside then stay context-free. Repeat
   identity on an event only when it can run *outside* that span — a spawned task, a callback (the
   sinks' delivery callbacks do this on purpose).
6. **No module prefix in the message.** The target already carries the module path, and
   `global.logger.modules` filters on it.
7. **Hot paths carry counters, not events.** Nothing per-record or per-packet above `trace`; the
   processor uses `InternalMetrics` instead.
8. **High-cardinality fields must be added to `with_excluded_fields`** in `init_logging`, or they
   blow the signature LRU.

### Dedicated streams

Some events use an explicit `target:` outside their module path so they can be filtered and
exported on their own: `patrolli::api::audit` (config mutations) and `patrolli::item_state`
(`cache::item::STATE_LOG_TARGET` — one `info` per item state change, emitted only through
`Transition::log` from what the `Stats::record_*` write paths return). `Stale` is derived on read
and deliberately not logged: detecting it would mean sweeping every item on a timer.

### Export

Logs can also be exported into the pipeline. `common::log_export` is a layer, installed in
`init_logging` after the fmt layer, that copies each matching event into the bounded flume channel
of every registration. `listener::internal_logs` (listener type `internal_logs`) is the consumer:
one registration per listener, shared by all its workers. It turns events into log records with
event fields and span fields as attributes.
- **`on_event` runs inside the caller's `warn!`**, so it never blocks, awaits, does I/O or logs. A
  full buffer drops the event and counts it in the listener's `total_items_dropped`.
- **INFO spans are always recorded**, so identity set on long-lived task spans survives until a
  listener registers. Spans below INFO are recorded only while a registration wants that level.
- **`log`-crate records** (rdkafka, fjall, rustls, tokio-postgres, refinery, …) reach the layer
  under tracing-log's placeholder target `log`; export matches and records them on
  `normalized_metadata()`. `log`'s max level is a separate global: `init_logging` derives it from
  the filters' `max_level_hint` (so every filter must report one — see `NoLevelBound` in
  `main.rs`), and registrations move it along in `rebuild_interest`.
- **Feedback loops.** The listener excludes its own module. Warnings from the path it sends
  through (`patrolli::listener`, the destination sink) can still come back through it. The export
  layer's own throttle instance and the bounded buffer keep that finite.

### API layer

Handlers do not log. They attach an operation label with `ApiResultExt` (`src/api/error.rs`) and let
the conversion emit one event:

```rust
let mut client = state.db_pool.get().await.ctx("get database connection")?;   // 500, logged once
payload.validate().bad_request("Validation error")?;                          // 400, logged at debug
```

`.ctx()` returns a generic `"Internal server error"` title — SQL, pool state and serde detail belong
in the log, not the response body. Explicit 4xx (`NOT_FOUND`, `CONFLICT`) are built inline and are
**not** logged; they are the caller's problem. `request_span` in `src/api/mod.rs` gives every line
`request_id`, `method`, `route` and `user`, so a handler event needs no context of its own.

## Gotchas

- Environment-only settings: `PATROLLI_JWT_SECRET` (≥32 bytes, identical on every cluster node,
  required in backend mode: with the API enabled, the node refuses to start without it), `PATROLLI_SECRET_KEYS`
  (`<id>:<secret>` entries, comma separated; PBKDF2 turns each secret of at least 16 bytes into the AES key) and `PATROLLI_SECRET_ACTIVE_KEY` (the id that encrypts, must be listed). Both
  must be identical on every cluster node and are required in backend mode, with or without the API. Without them, the node exits before it
  touches the database. `PATROLLI_ADMIN_USER` / `PATROLLI_ADMIN_PASSWORD` (seed the first user when
  the `users` table is empty), `PATROLLI_ADMIN_ALLOW_SSO` (`true` or `false`, default `false`), `PATROLLI_TEST_DATABASE_URL` (Postgres server for
the database tests instead of a container each), `PATROLLI_TEST_UPSTREAM` (`latest` to test the sinks
against the newest Kafka, Redpanda and ClickHouse instead of the pinned versions),
`PATROLLI_TEST_PANIC` (`processor`: the first processor batch panics; read by debug builds only, for
`tests/shutdown_test.rs`).
- SNMPv3 trap receiving (any USM user) and DES/3DES privacy (traps and polling) need
  async-snmp's authoritative engine state and DES salt state, which it refuses to build without.
  Both come from `common::snmp` (one per process): `snmpEngineBoots` is the seconds since
  2026-01-01 at first use, so it rises on every restart with nothing persisted.
- Heap profiling ships in the default `mem-profile` feature but boots idle. `SIGUSR1` arms sampling,
  each further `SIGUSR1` dumps `/tmp/patrolli.heap.<pid>.<seq>.prof`, `SIGUSR2` dumps and disarms; read
  the long comment on `install_mem_profile_signals` in `main.rs` before touching it. Dumps do not
  work on macOS — capture on Linux.
- `src/sink/clickhouse` is `pub` (unlike its sibling sinks) purely so integration tests can drive
  the DDL and the `Payload` → Arrow conversion directly. Its schema in `record_batch::schema` and its
  `ddl` must stay in sync; `tests/clickhouse_insert_test.rs` is what catches drift.
- `src/sink/kafka` is `pub` for the same reason: `tests/kafka_sink_test.rs` runs `KafkaSink` against
  Apache Kafka and Redpanda and reads its metrics after `close`. Its `flush` calls `rd_kafka_flush`
  directly because rust-rdkafka's `Producer::flush` never cuts `linger.ms` short (see the comment).
