// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Runs a draft processor chain over a payload without writing internal
//! metrics, lookup table counters or lookup tables.

use crate::{attribute_filter, attribute_lookup, attribute_transform, transform};
use anyhow::{Context, Result, anyhow, bail};
use otel_arrow_dfe_config::SignalType;
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::otap::batching::make_item_batches;
use patrolli::cache;
use patrolli::config::processor::{
    AddAttributes, Config, DropAttributes, Processor, RenameAttributes,
};
use patrolli::payload::Payload;
use std::num::NonZeroU64;
use std::sync::Arc;

pub struct Chain {
    processors: Vec<Arc<Processor>>,
    transforms: transform::Transforms,
}

impl Chain {
    pub fn new(processors: Vec<Arc<Processor>>) -> Self {
        Self {
            processors,
            transforms: transform::Transforms::default(),
        }
    }

    /// A failed lookup rule does not stop the chain, as in the workers. `run`
    /// returns the first such error next to the payload.
    pub fn run(
        &self,
        mut pl: Payload,
        cache: &cache::Cache,
    ) -> Result<(Payload, Option<anyhow::Error>)> {
        let mut skipped = None;
        for processor in &self.processors {
            if pl.is_empty() {
                break;
            }
            pl = self
                .run_processor(processor, pl, cache, &mut skipped)
                .with_context(|| format!("processor {}", processor.name))?;
        }
        Ok((pl, skipped))
    }

    fn run_processor(
        &self,
        processor: &Arc<Processor>,
        pl: Payload,
        cache: &cache::Cache,
        skipped: &mut Option<anyhow::Error>,
    ) -> Result<Payload> {
        match &processor.config {
            Config::Null => Ok(pl),
            Config::AttributeFilter(filter) => attribute_filter::attribute_filter(pl, filter),
            Config::DropAttributes(DropAttributes { transforms, .. })
            | Config::AddAttributes(AddAttributes { transforms, .. })
            | Config::RenameAttributes(RenameAttributes { transforms, .. }) => match transforms {
                Some(transforms) => attribute_transform::attribute_transform(pl, transforms),
                None => bail!("processor transforms not compiled"),
            },
            Config::AttributeLookup(config) => {
                let (pl, errored) = attribute_lookup::attribute_lookup(pl, config, None, cache);
                if errored && skipped.is_none() {
                    *skipped = Some(anyhow!(
                        "processor {}: attribute lookup skipped a rule, see the server log",
                        processor.name
                    ));
                }
                Ok(pl)
            }
            Config::AttributeCollect(_) => Ok(pl),
            Config::Transform(program) => self.transforms.run(processor, program, pl),
        }
    }
}

/// One payload per record: a log record, a span, or a metric with all its
/// data points.
pub fn split_records(pl: Payload) -> Result<Vec<Payload>> {
    let signal = match &pl.data {
        OtapArrowRecords::Logs(_) => SignalType::Logs,
        OtapArrowRecords::Metrics(_) => SignalType::Metrics,
        OtapArrowRecords::Traces(_) => SignalType::Traces,
    };
    let chunks =
        make_item_batches(signal, Some(NonZeroU64::MIN), vec![pl.data]).context("split records")?;
    Ok(chunks.into_iter().map(Payload::new).collect())
}
