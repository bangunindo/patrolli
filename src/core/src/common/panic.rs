// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Any panic is fatal. A panic means an invariant broke somewhere, and the
//! state it broke may be shared: `parking_lot` locks do not poison, so a
//! worker respawned in place could go on reading what the panicking one left
//! half written. Rather than respawn, the hook logs the panic, starts the
//! orderly shutdown (producers drain into consumers, sinks flush, disk topics
//! close) and the process exits with [`EXIT_CODE`], for the service manager
//! to start a fresh one. Only the batch the panicking thread held is lost.
//!
//! Nothing in the dependency tree panics and catches it as part of normal
//! operation, so a hook firing on every panic, caught or not, is safe.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;
use tracing::error;

/// Exit code of a process that panicked: `EX_SOFTWARE`, apart from the
/// shutdown watchdog's `1` and the signals' `130`/`143`.
pub const EXIT_CODE: i32 = 70;

static PANICKED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN: OnceLock<CancellationToken> = OnceLock::new();

/// Whether any thread has panicked since [`install`].
pub fn panicked() -> bool {
    PANICKED.load(Ordering::Relaxed)
}

/// The token a panic cancels: the root shutdown token, once the run function
/// has one. A panic before that still unwinds into `main`.
pub fn shutdown_on_panic(token: CancellationToken) {
    _ = SHUTDOWN.set(token);
}

/// Replaces the default hook, which writes to stderr only, with one that
/// logs through tracing, so the panic reaches the configured output and the
/// `internal_logs` export, then starts the shutdown. Call it once logging is
/// up.
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        PANICKED.store(true, Ordering::Relaxed);
        let thread = std::thread::current();
        // Empty unless `RUST_BACKTRACE` is set (`logger.print_trace`).
        let backtrace = Backtrace::capture();
        error!(
            panic = info.payload_as_str().unwrap_or("<non-string payload>"),
            location = info.location().map(tracing::field::display),
            thread = thread.name().unwrap_or("<unnamed>"),
            backtrace = (backtrace.status() == BacktraceStatus::Captured)
                .then(|| tracing::field::display(&backtrace)),
            "panicked, shutting down"
        );
        if let Some(token) = SHUTDOWN.get() {
            token.cancel();
        }
    }));
}
