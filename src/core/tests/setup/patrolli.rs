// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! A `patrolli` daemon run from the binary cargo builds for the integration tests,
//! for the end-to-end tests. No image to build: the process is the working tree.

#![allow(dead_code)]

use parking_lot::Mutex;
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

/// How long [`Patrolli::stop`] waits for the process to exit after SIGTERM before
/// killing it.
const STOP_LIMIT: Duration = Duration::from_secs(10);

type Lines = Arc<Mutex<Vec<String>>>;

/// One `patrolli --config <dir>/config.yaml` process, with `<dir>` a fresh temp
/// directory that is also its working directory, so `data_dir: ./data` lands
/// there. Every output line is kept for the `*_contains` checks and echoed with
/// a `[name]` prefix; the echo goes through libtest's capture, so it only
/// shows when the test fails (or with `--nocapture`).
pub struct Patrolli {
    name: String,
    child: Child,
    stdout: Lines,
    stderr: Lines,
    _dir: tempfile::TempDir,
}

impl Patrolli {
    pub fn start(name: &str, config_yaml: &str, envs: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("failed to create the patrolli temp dir");
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, config_yaml).expect("failed to write the patrolli config");
        let mut child = Command::new(env!("CARGO_BIN_EXE_patrolli"))
            .arg("--config")
            .arg(&path)
            .current_dir(dir.path())
            .env("PATROLLI_SECRET_KEYS", super::SECRET_KEYS)
            .env("PATROLLI_SECRET_ACTIVE_KEY", super::SECRET_ACTIVE_KEY)
            .envs(envs.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn patrolli");
        let stdout = read_lines(name, child.stdout.take().unwrap());
        let stderr = read_lines(name, child.stderr.take().unwrap());
        Self {
            name: name.to_string(),
            child,
            stdout,
            stderr,
            _dir: dir,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether any stdout line so far contains `needle` (where the `stdout`
    /// file sink writes).
    pub fn stdout_contains(&self, needle: &str) -> bool {
        self.stdout.lock().iter().any(|l| l.contains(needle))
    }

    /// Whether any stderr line so far contains `needle` (where the log goes).
    pub fn stderr_contains(&self, needle: &str) -> bool {
        self.stderr.lock().iter().any(|l| l.contains(needle))
    }

    /// SIGTERM, then wait up to [`STOP_LIMIT`] for the exit before killing it.
    pub async fn stop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
        let deadline = tokio::time::Instant::now() + STOP_LIMIT;
        while tokio::time::Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        println!(
            "[{}] still running {STOP_LIMIT:?} after SIGTERM, killing it",
            self.name
        );
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Patrolli {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_lines(name: &str, pipe: impl Read + Send + 'static) -> Lines {
    let lines = Lines::default();
    let sink = lines.clone();
    let name = name.to_string();
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            println!("[{name}] {line}");
            sink.lock().push(line);
        }
    });
    lines
}

/// A TCP port nothing listens on right now. Another process can still take it
/// before the caller binds it, which is rare enough for tests.
pub fn free_tcp_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|s| s.local_addr())
        .expect("failed to find a free tcp port")
        .port()
}

/// A UDP port nothing is bound to right now; see [`free_tcp_port`].
pub fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .and_then(|s| s.local_addr())
        .expect("failed to find a free udp port")
        .port()
}
