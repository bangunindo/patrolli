// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

pub mod backoff;
pub mod health;
pub mod internal_metrics;
pub mod log_export;
pub mod mib;
pub mod otap_client;
pub mod panic;
pub mod prometheus;
pub mod serde;
pub mod shutdown;
pub mod snmp;
pub mod tls;
mod udp_listener;

use chrono::{DateTime, FixedOffset, Offset, TimeZone, Utc};
use chrono_tz::Tz;
use std::env;
use std::net::IpAddr;
use std::sync::LazyLock;
use tokio::time::{Duration, Instant};
pub use udp_listener::{ByteCounters, ByteCounts, UdpListenerWorker};

pub static DEFAULT_TZ: LazyLock<Tz> = LazyLock::new(|| {
    env::var("TZ")
        .unwrap_or_else(|_| "UTC".to_string())
        .parse()
        .unwrap_or(chrono_tz::UTC)
});

pub fn now() -> DateTime<Tz> {
    Utc::now().with_timezone(&DEFAULT_TZ)
}

static START_TIME: LazyLock<Instant> = LazyLock::new(Instant::now);

static START_TIME_TZ: LazyLock<DateTime<Tz>> = LazyLock::new(now);

pub fn uptime() -> Duration {
    START_TIME.elapsed()
}

pub fn get_start_datetime() -> DateTime<Tz> {
    *START_TIME_TZ
}

pub fn truncate_instant(instant: Instant, precision: Duration) -> Instant {
    let elapsed = instant.duration_since(*START_TIME);
    let truncated_duration = truncate_duration(elapsed, precision);
    *START_TIME + truncated_duration
}

pub fn truncate_duration(d: Duration, precision: Duration) -> Duration {
    let nanos = d.as_nanos();
    let precision_nanos = precision.as_nanos();
    Duration::from_nanos(((nanos / precision_nanos) * precision_nanos) as u64)
}

pub fn datetime_to_instant(dt: DateTime<Tz>) -> Option<Instant> {
    let now_chrono = now();
    let now_instant = Instant::now();

    if dt >= now_chrono {
        let duration = (dt - now_chrono).to_std().ok()?;
        now_instant.checked_add(duration)
    } else {
        let duration = (now_chrono - dt).to_std().ok()?;
        now_instant.checked_sub(duration)
    }
}

pub fn guess_tz_from_offset(dt: DateTime<FixedOffset>) -> DateTime<Tz> {
    use chrono_tz::TZ_VARIANTS;

    let offset_secs = dt.offset().local_minus_utc();
    let naive_utc = dt.naive_utc();

    // Find first Tz whose UTC offset matches at this specific instant
    let tz = TZ_VARIANTS
        .iter()
        .find(|&&tz| {
            tz.offset_from_utc_datetime(&naive_utc)
                .fix()
                .local_minus_utc()
                == offset_secs
        })
        .copied()
        .unwrap_or(chrono_tz::UTC);

    tz.from_utc_datetime(&naive_utc)
}

pub fn ip_allowed(ip: &IpAddr, whitelist: &[IpAddr], blacklist: &[IpAddr]) -> bool {
    if !whitelist.is_empty() && whitelist.binary_search(ip).is_err() {
        return false;
    }
    if !blacklist.is_empty() && blacklist.binary_search(ip).is_ok() {
        return false;
    }
    true
}

pub trait HasLength {
    fn len(&self) -> usize;
    /// Defaults to `len() == 0`, but implementors may override: for payloads
    /// `len()` counts items (data points for metrics) while emptiness is a
    /// question about records. Any wrapper impl must forward this rather than
    /// inherit the default, or it silently reintroduces the item-count meaning.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: HasLength + ?Sized> HasLength for std::sync::Arc<T> {
    fn len(&self) -> usize {
        (**self).len()
    }

    fn is_empty(&self) -> bool {
        (**self).is_empty()
    }
}
