// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The `transform` processor: runs a processor's program (OPL) through
//! otel-arrow's columnar query engine.
//!
//! An engine `Pipeline` plans itself on its first `execute` (from the program
//! alone; the batch does not shape the plan) and runs behind `&mut self`. It
//! is not `Send` either, so it cannot live in the shared, lock-free route
//! tree: each processor worker keeps its own per processor instead, rebuilt
//! when the processor's config is replaced.

use crate::config::processor::{self, transform};
use crate::payload::Payload;
use anyhow::{Context, Result};
use otel_arrow_dfe_config::SignalType;
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::otap::transform::sanitize::sanitize_otap_batch;
use otel_arrow_dfe_query_engine::pipeline::{Pipeline as Engine, PipelineOptions};
use rustc_hash::FxHashMap;
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use uuid::Uuid;

/// One worker's engines for every `transform` processor it has run.
#[derive(Default)]
pub(super) struct Transforms {
    entries: RefCell<FxHashMap<Uuid, Entry>>,
    /// Address of the route tree the entries were last pruned against.
    pruned_at: Cell<usize>,
}

struct Entry {
    /// The config the engine was built from. Every upsert replaces the
    /// `Arc`, so a pointer mismatch means the program changed. Holding it
    /// also keeps the address from being reused while the entry lives.
    config: Arc<processor::Processor>,
    engine: Engine,
}

const fn signal_type(data: &OtapArrowRecords) -> SignalType {
    match data {
        OtapArrowRecords::Logs(_) => SignalType::Logs,
        OtapArrowRecords::Metrics(_) => SignalType::Metrics,
        OtapArrowRecords::Traces(_) => SignalType::Traces,
    }
}

impl Transforms {
    /// Drop the entries of processors that are gone, once per route tree:
    /// `tree` is the address of the tree being read, `live` its processor ids.
    pub(super) fn prune(&self, tree: usize, live: impl FnOnce() -> Vec<Uuid>) {
        if self.pruned_at.replace(tree) == tree {
            return;
        }
        let mut entries = self.entries.borrow_mut();
        if entries.is_empty() {
            return;
        }
        let live = live();
        entries.retain(|id, _| live.contains(id));
    }

    /// Run `config`'s program over `pl`. A batch of a signal the program does
    /// not select comes back unchanged.
    pub(super) fn run(
        &self,
        config: &Arc<processor::Processor>,
        transform: &transform::Transform,
        pl: Payload,
    ) -> Result<Payload> {
        if pl.is_empty() {
            return Ok(pl);
        }
        let mut entries = self.entries.borrow_mut();
        let entry = match entries.get_mut(&config.id) {
            Some(entry) if Arc::ptr_eq(&entry.config, config) => entry,
            _ => {
                let (expr, signal_context) = transform.compile()?;
                let options = PipelineOptions {
                    signal_context,
                    ..Default::default()
                };
                let entry = Entry {
                    config: Arc::clone(config),
                    engine: Engine::new_with_options(expr, options),
                };
                entries.insert(config.id, entry);
                entries.get_mut(&config.id).expect("just inserted")
            }
        };
        if !entry.engine.accepts_signal_type(signal_type(&pl.data)) {
            return Ok(pl);
        }
        // Nothing the engine does today waits, so this never parks. It has no
        // reactor: an engine stage needing tokio's timers, I/O or spawn would
        // panic here, and would need a runtime on the worker thread instead.
        let mut data = futures::executor::block_on(entry.engine.execute(pl.data))
            .context("execute transform program")?;
        // `where`/`drop`/`remove` rewrite dictionary keys but keep the
        // dictionary values, so what they removed would still ride along in
        // Arrow IPC (OTAP sink, disk channel, forwarder). This keeps only the
        // values some key still references.
        sanitize_otap_batch(&mut data);
        Ok(Payload::new(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::processor::Config;
    use crate::processor::test_payloads::{attr_strs, logs_with_unowned_resource, signals};
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;

    fn processor(opl: &str) -> Arc<processor::Processor> {
        let mut processor: processor::Processor = serde_json::from_value(serde_json::json!({
            "id": Uuid::now_v7(),
            "name": "t",
            "type": "transform",
            "config": {"opl": opl},
        }))
        .unwrap();
        processor.validate().unwrap();
        Arc::new(processor)
    }

    fn run(
        transforms: &Transforms,
        config: &Arc<processor::Processor>,
        pl: Payload,
    ) -> Result<Payload> {
        let Config::Transform(transform) = &config.config else {
            unreachable!()
        };
        transforms.run(config, transform, pl)
    }

    fn logs() -> Payload {
        signals().remove(0)
    }

    #[test]
    fn set_adds_an_attribute_to_every_log_record() {
        let transforms = Transforms::default();
        let config = processor(r#"logs | set attributes["opl"] = "yes""#);
        let out = run(&transforms, &config, logs()).unwrap();
        assert_eq!(out.num_items(), 2);
        assert_eq!(
            attr_strs(&out, ArrowPayloadType::LogAttrs, "opl"),
            ["yes", "yes"]
        );
    }

    #[test]
    fn where_drops_the_records_that_do_not_match() {
        let transforms = Transforms::default();
        let keep = processor(r#"logs | where resource.attributes["host"] == "a""#);
        assert_eq!(run(&transforms, &keep, logs()).unwrap().num_items(), 2);
        let none = processor(r#"logs | where resource.attributes["host"] == "b""#);
        assert_eq!(run(&transforms, &none, logs()).unwrap().num_items(), 0);
    }

    #[test]
    fn a_batch_of_an_unselected_signal_passes_through() {
        let transforms = Transforms::default();
        let config = processor(r#"logs | where resource.attributes["host"] == "b""#);
        let mut payloads = signals();
        let traces = payloads.pop().unwrap();
        let metrics = payloads.pop().unwrap();
        assert_eq!(run(&transforms, &config, metrics).unwrap().num_items(), 2);
        assert_eq!(run(&transforms, &config, traces).unwrap().num_items(), 2);
    }

    #[test]
    fn metrics_and_traces_are_transformed_too() {
        let transforms = Transforms::default();
        let mut payloads = signals();
        let traces = payloads.pop().unwrap();
        let metrics = payloads.pop().unwrap();
        let config = processor(r#"metrics | set attributes["opl"] = "m""#);
        let out = run(&transforms, &config, metrics).unwrap();
        assert_eq!(
            attr_strs(&out, ArrowPayloadType::MetricAttrs, "opl"),
            ["m", "m"]
        );
        let config = processor(r#"traces | set attributes["opl"] = "t""#);
        let out = run(&transforms, &config, traces).unwrap();
        assert_eq!(
            attr_strs(&out, ArrowPayloadType::SpanAttrs, "opl"),
            ["t", "t"]
        );
    }

    #[test]
    fn one_signals_engine_transforms_all_three_signals() {
        let transforms = Transforms::default();
        let config = processor(r#"signals | set resource.attributes["opl"] = "s""#);
        for (pl, signal) in signals().into_iter().zip(["logs", "metrics", "traces"]) {
            let out = run(&transforms, &config, pl).unwrap_or_else(|e| panic!("{signal}: {e:#}"));
            assert_eq!(
                attr_strs(&out, ArrowPayloadType::ResourceAttrs, "opl"),
                ["s"],
                "{signal}"
            );
        }
        assert_eq!(transforms.entries.borrow().len(), 1);
    }

    #[test]
    fn apply_data_points_reaches_data_point_attributes() {
        let transforms = Transforms::default();
        let metrics = signals().remove(1);
        let config = processor(r#"metrics | apply data_points { set attributes["dp"] = "x" }"#);
        let out = run(&transforms, &config, metrics).unwrap();
        assert_eq!(
            attr_strs(&out, ArrowPayloadType::NumberDpAttrs, "dp"),
            ["x", "x"]
        );
    }

    #[test]
    fn one_engine_handles_batches_of_different_shapes() {
        // One engine sees every batch of its processor, and later batches have
        // other columns and attribute tables than the one it planned on.
        let transforms = Transforms::default();
        let config = processor(r#"logs | set attributes["opl"] = "yes""#);
        let first = run(&transforms, &config, logs()).unwrap();
        assert_eq!(
            attr_strs(&first, ArrowPayloadType::LogAttrs, "opl").len(),
            2
        );
        for null_struct in [false, true] {
            let out = run(
                &transforms,
                &config,
                logs_with_unowned_resource(null_struct),
            )
            .unwrap_or_else(|e| panic!("null_struct={null_struct}: {e:#}"));
            assert_eq!(
                attr_strs(&out, ArrowPayloadType::LogAttrs, "opl"),
                ["yes", "yes"],
                "null_struct={null_struct}"
            );
        }
        let again = run(&transforms, &config, logs()).unwrap();
        assert_eq!(
            attr_strs(&again, ArrowPayloadType::LogAttrs, "opl").len(),
            2
        );
    }

    #[test]
    fn a_filter_planned_on_a_batch_without_record_attributes_still_matches_later() {
        // `logs()` has no log attribute table at all; the next batch has one.
        let transforms = Transforms::default();
        let config = processor(r#"logs | where attributes["observed_site"] == "correct""#);
        assert_eq!(run(&transforms, &config, logs()).unwrap().num_items(), 0);
        let out = run(&transforms, &config, logs_with_unowned_resource(false)).unwrap();
        assert_eq!(out.num_items(), 1);
        assert_eq!(
            attr_strs(&out, ArrowPayloadType::LogAttrs, "observed_site"),
            ["correct"]
        );
    }

    #[test]
    fn a_replaced_config_rebuilds_the_engine_and_prune_drops_deleted_ones() {
        let transforms = Transforms::default();
        let old = processor(r#"logs | set attributes["v"] = "old""#);
        run(&transforms, &old, logs()).unwrap();
        let mut replaced = (*old).clone();
        replaced.config = processor(r#"logs | set attributes["v"] = "new""#)
            .config
            .clone();
        let replaced = Arc::new(replaced);
        let out = run(&transforms, &replaced, logs()).unwrap();
        assert_eq!(
            attr_strs(&out, ArrowPayloadType::LogAttrs, "v"),
            ["new", "new"]
        );

        transforms.prune(1, || vec![replaced.id]);
        assert_eq!(transforms.entries.borrow().len(), 1);
        // Same tree again: no rescan, even if the live set would differ.
        transforms.prune(1, Vec::new);
        assert_eq!(transforms.entries.borrow().len(), 1);
        transforms.prune(2, Vec::new);
        assert!(transforms.entries.borrow().is_empty());
    }
}
