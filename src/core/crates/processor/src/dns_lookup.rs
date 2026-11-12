// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The `dns_lookup` processor: DNS answers from `cache::dns` written to
//! attributes. Config contract on [`patrolli::config::processor::dns_lookup`].
//!
//! The input side is `attribute_lookup`'s (key fallbacks, regex extraction,
//! the record-reads-resource indirection, `when` guards), and so is the write
//! side ([`super::attr_write`]); only the probe differs. It never waits: the
//! cache answers from memory, and a question it cannot answer is queued for
//! the resolver and found by a later batch.
//!
//! Each distinct question is put to the cache once per batch, whatever the
//! number of records and rules asking it: the cache counts lookups, and a
//! batch of flows repeats a handful of addresses thousands of times.

use super::attr_input;
use super::attr_write::{Values, Writes, apply};
use super::attribute_lookup::{Extractor, probe_address};
use anyhow::Result;
use arrow::array::BooleanArray;
use patrolli::cache;
use patrolli::cache::dns::{Answer, Question, Reader};
use patrolli::config::processor::dns_lookup::{DnsLookup, DnsLookupRule, DnsRecordType};
use patrolli::payload::{Payload, attr};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use tracing::warn;

/// Enrich `pl` in place and report whether any rule was skipped by an error.
/// No failing return: the payload always comes back, enriched as far as it got.
/// `counted: false` (a preview) leaves the cache's lookup counters alone.
pub(super) fn dns_lookup(
    mut pl: Payload,
    config: &DnsLookup,
    cache: &cache::Cache,
    counted: bool,
) -> (Payload, bool) {
    if pl.is_empty() {
        return (pl, false);
    }
    let mut errored = false;
    let shared = match &config.when {
        None => None,
        Some(filter) => match attr::attribute_filter_mask(&pl, filter) {
            Ok(mask) => Some(mask),
            Err(e) => {
                warn!(error = %format!("{e:#}"), "shared `when` failed, skipping enrichment");
                return (pl, true);
            }
        },
    };

    let mut writes = Writes::default();
    {
        let mut answers = Answers {
            reader: if counted {
                cache.dns.reader()
            } else {
                cache.dns.uncounted_reader()
            },
            memo: FxHashMap::default(),
        };
        for (i, rule) in config.lookups.iter().enumerate() {
            if let Err(e) = run_rule(&pl, rule, shared.as_ref(), &mut answers, &mut writes) {
                errored = true;
                warn!(error = %format!("{e:#}"), lookup = i, "lookup skipped");
            }
        }
    }

    if let Err(e) = apply(&mut pl, writes, config.overwrite) {
        errored = true;
        warn!(error = %format!("{e:#}"), "writing enriched attributes failed");
    }
    (pl, errored)
}

/// The cache, asked at most once per question per batch.
struct Answers<'a> {
    reader: Reader<'a>,
    memo: FxHashMap<Question, Option<Answer>>,
}

impl Answers<'_> {
    fn get(&mut self, question: Question) -> Option<&Answer> {
        let reader = &mut self.reader;
        self.memo
            .entry(question)
            .or_insert_with(|| reader.lookup(&question))
            .as_ref()
    }
}

fn run_rule(
    pl: &Payload,
    rule: &DnsLookupRule,
    shared: Option<&BooleanArray>,
    answers: &mut Answers<'_>,
    writes: &mut Writes,
) -> Result<()> {
    let parent_ids = attr_input::rule_parent_ids(pl, &rule.table)?;
    let eligible = attr_input::eligible_parents(pl, shared, rule.when.as_ref(), &parent_ids)?;
    let is_eligible = |parent| eligible.as_ref().is_none_or(|set| set.contains(&parent));

    // An explicit null is not an address, so it reads as absent.
    let read = |key: &String| {
        attr_input::read_source(
            pl,
            &rule.table,
            rule.input.table.as_ref(),
            key,
            &parent_ids,
            false,
        )
    };
    let source = match rule.input.key.as_slice() {
        [key] => read(key)?,
        keys => attr_input::Source::FirstOf(keys.iter().map(read).collect::<Result<_>>()?),
    };
    let mut extractor = Extractor::new(&rule.input);

    let mut names = FxHashMap::default();
    for parent in attr_input::candidates(std::iter::once(&source), is_eligible) {
        // Not an address: nothing to ask, like a missing input.
        let Some(ip) = source
            .get(parent)
            .and_then(|value| probe_address(value, extractor.as_mut()))
        else {
            continue;
        };
        let question = match rule.record {
            DnsRecordType::Ptr => Question::Ptr(ip),
        };
        // A negative or failed entry, a miss and a skipped question all
        // leave the destination alone.
        if let Some(Answer::Name(name)) = answers.get(question) {
            names.insert(parent, name.clone());
        }
    }
    if !names.is_empty() {
        // Destinations are unique per table (validated), so this rule owns
        // the entry.
        writes
            .table(&rule.table)
            .insert(SmolStr::new(&rule.output), Values::Str(names));
    }
    Ok(())
}
