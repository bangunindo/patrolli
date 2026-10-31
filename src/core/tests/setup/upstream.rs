// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Tags for the third-party servers the sink tests run against (Kafka, Redpanda,
//! ClickHouse). Its own file because `bench_kafka` includes `setup/kafka.rs`
//! without the rest of `setup`.

/// `pinned`, the version the suite was last verified with, unless
/// `PATROLLI_TEST_UPSTREAM=latest` asks for `latest`. The pin keeps a push's CI
/// from going red over an upstream release; the override is for a scheduled job
/// that should catch a release breaking a sink before anyone bumps the pin.
pub fn upstream_tag(pinned: &'static str) -> &'static str {
    match std::env::var("PATROLLI_TEST_UPSTREAM").as_deref() {
        Ok("latest") => "latest",
        Ok("") | Err(_) => pinned,
        Ok(other) => panic!("PATROLLI_TEST_UPSTREAM={other:?}: expected `latest` or unset"),
    }
}
