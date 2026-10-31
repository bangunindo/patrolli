// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! SIGTERM with a sink whose destination is down. The sink retries forever,
//! so its topic fills, then the processors block on it, then the processor
//! channel fills and the monitors and listeners block on that. Shutdown used
//! to wait on all of them in turn and never end; it must now end by
//! `global.shutdown_timeout`, and a second signal must end it at once.
//!
//! Runs the real `patrolli` binary (cargo builds it for integration tests) in
//! local mode, so no Docker. The sink points at 127.0.0.1:1, which refuses
//! the connection, and nothing binds a port, so the tests run in parallel.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SINK_ID: &str = "019b6e6e-7e75-70a2-b0ba-d88cb64a9d4c";
/// A debug build loading the system MIBs, on a busy machine.
const STARTUP_LIMIT: Duration = Duration::from_secs(60);
/// Long enough for every stage to back up: the dummy listener sends 200
/// batches a second and the dummy item runs every second.
const BACKUP_TIME: Duration = Duration::from_secs(3);
/// What `main.rs` waits past the deadline before its watchdog exits.
const WATCHDOG_GRACE: Duration = Duration::from_secs(5);

/// `sink_channel` is the sink's `channel:` block, indented under it.
fn config(shutdown_timeout_ms: u64, sink_channel: &str) -> String {
    format!(
        r#"global:
  shutdown_timeout: {shutdown_timeout_ms}
  logger:
    level: info
    disable_color: true
  mib:
    disabled: true
  disk_channel:
    data_dir: ./data
listeners:
  - type: dummy
    id: 019b77b3-df79-70a2-ab01-f43b2cf723a0
    name: dummy-1
    worker_count: 1
    config:
      speed: 200
      messages_per_batch: 100
hosts:
  - id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    name: h1
processors:
  - id: 01a05ad4-2be6-70a2-abf4-e50b1d88d31e
    name: passthrough
    type: "null"
items:
  - type: listener
    id: 019b914b-7234-70a2-af38-89569b7c1b0f
    name: dummy-listener-item
    listener_id: 019b77b3-df79-70a2-ab01-f43b2cf723a0
    host_id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    processors:
      - 01a05ad4-2be6-70a2-abf4-e50b1d88d31e
    config:
      match_strategy:
        method: none
  - type: dummy
    id: 019c65dc-6f10-70a2-ad19-b7c6eadeff92
    name: dummy-monitor-item
    host_id: 019bcb00-2fee-70a2-b151-3b1c2de7d947
    schedule: "1"
    processors:
      - 01a05ad4-2be6-70a2-abf4-e50b1d88d31e
    config:
      count: 10
      payload:
        row:
          body: {{String: hello}}
          attrs: []
sinks:
  - type: open_telemetry
    id: {SINK_ID}
    name: unreachable
    worker_count: 1
    batch_size: 1000
    batch_timeout: 1000
    channel:
{sink_channel}
    inputs:
      - 01a05ad4-2be6-70a2-abf4-e50b1d88d31e
    config:
      destinations:
        - 127.0.0.1:1
"#
    )
}

const MEMORY_CHANNEL: &str = "      type: memory
      non_blocking: false
      channel_buffer_size: 0";

/// Full once it holds anything at all.
const DISK_CHANNEL: &str = "      type: disk
      non_blocking: false
      max_disk_size: 1
      channel_buffer_size: 0";

struct Daemon {
    child: Child,
    log: Arc<Mutex<Vec<String>>>,
    dir: tempfile::TempDir,
}

impl Daemon {
    fn start(config: &str) -> Self {
        Self::start_with_env(config, &[])
    }

    fn start_with_env(config: &str, env: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, config).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_patrolli"))
            .envs(env.iter().copied())
            .arg("--config")
            .arg(&path)
            .current_dir(dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = child.stderr.take().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let lines = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                lines.lock().unwrap().push(line);
            }
        });
        Self { child, log, dir }
    }

    fn logged(&self, needle: &str) -> bool {
        self.log.lock().unwrap().iter().any(|l| l.contains(needle))
    }

    fn dump(&self) -> String {
        self.log.lock().unwrap().join("\n")
    }

    fn wait_for_log(&self, needle: &str, limit: Duration) {
        let started = Instant::now();
        while !self.logged(needle) {
            assert!(
                started.elapsed() < limit,
                "no {needle:?} within {limit:?}; log:\n{}",
                self.dump()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Starts it and waits until the sink is failing and everything in
    /// front of it has had time to back up.
    fn start_backed_up(config: &str) -> Self {
        let daemon = Self::start(config);
        daemon.wait_for_log("startup complete", STARTUP_LIMIT);
        daemon.wait_for_log("destination unavailable, retrying", STARTUP_LIMIT);
        std::thread::sleep(BACKUP_TIME);
        daemon
    }

    fn signal(&self, signal: libc::c_int) {
        let res = unsafe { libc::kill(self.child.id() as libc::pid_t, signal) };
        assert_eq!(res, 0, "kill failed");
    }

    fn wait_exit(&mut self, limit: Duration) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            if started.elapsed() >= limit {
                let _ = self.child.kill();
                panic!(
                    "still running {limit:?} after the signal; log:\n{}",
                    self.dump()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The log, once the reader thread has seen the pipe close.
    fn final_log(&self) -> String {
        std::thread::sleep(Duration::from_millis(200));
        self.dump()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// SIGTERM, then the exit status and how long it took.
fn terminate(daemon: &mut Daemon, timeout: Duration) -> (ExitStatus, Duration) {
    let started = Instant::now();
    daemon.signal(libc::SIGTERM);
    // Past the watchdog: a hang is reported as one, not as a slow exit.
    let status = daemon.wait_exit(timeout + WATCHDOG_GRACE * 2);
    (status, started.elapsed())
}

#[test]
fn a_sink_that_stopped_consuming_does_not_hold_shutdown() {
    let timeout = Duration::from_secs(6);
    let mut daemon = Daemon::start_backed_up(&config(timeout.as_millis() as u64, MEMORY_CHANNEL));
    let (status, took) = terminate(&mut daemon, timeout);
    let log = daemon.final_log();
    // 0, not the watchdog's 1: the teardown itself ended in time.
    assert_eq!(status.code(), Some(0), "{status}; log:\n{log}");
    // The deadline, plus the processors' and the sink's grace for handing
    // over what they hold, plus scheduling.
    assert!(
        took < timeout + Duration::from_secs(4),
        "took {took:?}; log:\n{log}"
    );
    for needle in [
        // the monitor workers and batcher no longer wait on a full channel
        "monitor batcher stopped",
        "processor drain timed out, no longer waiting on full sink topics",
        "records dropped on shutdown, sink not consuming",
        "shutdown complete",
    ] {
        assert!(log.contains(needle), "no {needle:?}; log:\n{log}");
    }
    assert!(
        !log.contains("processor workers did not stop"),
        "log:\n{log}"
    );
}

#[test]
fn a_full_disk_topic_takes_what_the_processors_hold_on_shutdown() {
    let timeout = Duration::from_secs(6);
    let mut daemon = Daemon::start_backed_up(&config(timeout.as_millis() as u64, DISK_CHANNEL));
    assert!(daemon.logged("disk channel full, applying backpressure"));
    let (status, took) = terminate(&mut daemon, timeout);
    let log = daemon.final_log();
    assert_eq!(status.code(), Some(0), "{status}; log:\n{log}");
    assert!(
        took < timeout + Duration::from_secs(4),
        "took {took:?}; log:\n{log}"
    );
    assert!(log.contains("shutdown complete"), "log:\n{log}");
    // Past its cap rather than dropped.
    assert!(!log.contains("records dropped on shutdown"), "log:\n{log}");

    let db = fjall::Database::builder(daemon.dir.path().join("data"))
        .open()
        .unwrap();
    let keyspace = db
        .keyspace(SINK_ID, fjall::KeyspaceCreateOptions::default)
        .unwrap();
    // The cap lets one payload in; the rest got there on shutdown.
    let kept = keyspace.iter().count();
    assert!(kept > 1, "{kept} payloads kept; log:\n{log}");
}

#[test]
fn a_second_signal_exits_at_once() {
    let timeout = Duration::from_secs(60);
    let mut daemon = Daemon::start_backed_up(&config(timeout.as_millis() as u64, MEMORY_CHANNEL));
    daemon.signal(libc::SIGTERM);
    daemon.wait_for_log("shutting down", Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(500));
    let started = Instant::now();
    daemon.signal(libc::SIGTERM);
    let status = daemon.wait_exit(Duration::from_secs(5));
    let took = started.elapsed();
    let log = daemon.final_log();
    assert_eq!(status.code(), Some(143), "{status}; log:\n{log}");
    assert!(took < Duration::from_secs(2), "took {took:?}; log:\n{log}");
    // Flushed by the forced exit, not lost with the log writer's buffer.
    assert!(
        log.contains("second shutdown signal, exiting now"),
        "log:\n{log}"
    );
    assert!(!log.contains("shutdown complete"), "log:\n{log}");
}

/// A panic is fatal, but not abrupt: the hook logs it and starts the same
/// orderly shutdown a signal does, and the process exits with 70 for the
/// service manager to restart it. `PATROLLI_TEST_PANIC` is read by debug builds
/// only; it makes the processor worker that takes the first batch panic.
#[test]
fn a_panic_shuts_down_in_order_and_exits_70() {
    let timeout = Duration::from_secs(6);
    let mut daemon = Daemon::start_with_env(
        &config(timeout.as_millis() as u64, MEMORY_CHANNEL),
        &[("PATROLLI_TEST_PANIC", "processor")],
    );
    daemon.wait_for_log("panicked, shutting down", STARTUP_LIMIT);
    let status = daemon.wait_exit(timeout + WATCHDOG_GRACE * 2);
    let log = daemon.final_log();
    assert_eq!(status.code(), Some(70), "{status}; log:\n{log}");
    let panics: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("panicked, shutting down"))
        .collect();
    assert_eq!(panics.len(), 1, "log:\n{log}");
    // Through tracing, with what and where.
    for needle in [
        "processor panic requested by PATROLLI_TEST_PANIC",
        "location=",
    ] {
        assert!(
            panics[0].contains(needle),
            "no {needle:?} in {:?}",
            panics[0]
        );
    }
    // The teardown ran to the end, past the step whose task panicked.
    for needle in [
        "shutting down",
        "monitor batcher stopped",
        "shutdown complete",
    ] {
        assert!(log.contains(needle), "no {needle:?}; log:\n{log}");
    }
    assert!(!log.contains("shutdown step failed"), "log:\n{log}");
}
