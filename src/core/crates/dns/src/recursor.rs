// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Recursive resolution from the root servers, in process.
//!
//! The recursor has no timeout, attempt or concurrency settings of its own:
//! [`crate::Router`] bounds each lookup with `lookup_timeout`, and the run
//! loop bounds concurrency with `max_inflight`. It caches no failures either;
//! `cache::dns` does.

use crate::Resolve;
use crate::outcome;
use anyhow::{Context, Result};
use futures::future::BoxFuture;
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::op::Query;
use hickory_resolver::recursor::{Recursor, RecursorError, RecursorOptions};
use patrolli::cache::dns::{Outcome, Question};
use patrolli::config::dns::Dns as DnsConfig;
use std::time::Instant;
use tracing::debug;

pub(crate) struct RecursorSource {
    recursor: Recursor<TokioRuntimeProvider>,
}

impl RecursorSource {
    pub(crate) fn new(config: &DnsConfig) -> Result<Self> {
        let options = RecursorOptions {
            ns_cache_size: config.recursor_ns_cache_size,
            response_cache_size: config.recursor_response_cache_size,
            // Depth counts per label, and an ip6.arpa name has 34: the
            // default of 24 fails every IPv6 reverse lookup.
            recursion_limit: config.recursion_limit,
            ns_recursion_limit: config.recursion_limit,
            ..RecursorOptions::default()
        };
        let recursor = Recursor::with_options(
            &config.root_servers,
            options,
            TokioRuntimeProvider::default(),
        )
        .context("build the DNS recursor")?;
        Ok(Self { recursor })
    }
}

impl Resolve for RecursorSource {
    fn resolve<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Outcome> {
        Box::pin(async move {
            let (name, record_type) = outcome::query(question);
            match self
                .recursor
                .resolve(Query::query(name, record_type), Instant::now(), false)
                .await
            {
                Ok(message) => {
                    outcome::from_records(question, &message.answers, &message.authorities)
                }
                Err(error) => from_error(error),
            }
        })
    }
}

fn from_error(error: RecursorError) -> Outcome {
    if error.is_nx_domain() || error.is_no_records_found() {
        let ttl = error
            .into_soa()
            .map(|soa| outcome::negative_ttl(soa.ttl, &soa.data));
        return Outcome::Negative { ttl };
    }
    debug!(error = %error, "recursive lookup failed");
    Outcome::Failed
}
