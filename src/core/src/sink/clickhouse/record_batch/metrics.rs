// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! `OtapArrowRecords::Metrics` → the five `otel_metrics_*` record batches.
//!
//! This is the signal the upstream Rust exporter does not implement (its metric DDL is a
//! literal `SELECT 1;` stub), and the reason is a grain mismatch that its parent→child
//! transform ops cannot express:
//!
//! - OTAP's `UnivariateMetrics` batch is one row per **metric** — name, unit, description,
//!   temporality, monotonicity, resource and scope.
//! - Every `otel_metrics_*` table is one row per **data point**.
//!
//! So the metric-level columns have to be broadcast *downwards*, from a handful of metric
//! rows to many data-point rows. That is precisely a gather: build one index mapping each
//! data-point row to its metric's row, then `take` every metric-level column through it. One
//! kernel call per column, no per-row work, and the resource/scope attribute maps ride along
//! as ordinary columns.
//!
//! Two further wrinkles the reference implementation calls out as blockers:
//!
//! - **One payload fans out to two tables.** Gauge and sum data points share a single
//!   `NumberDataPoints` batch and are told apart only by the parent metric's `metric_type`.
//!   After the broadcast that type is available at data-point grain, so the split is a
//!   `filter_record_batch` on each value.
//! - **Data-point attributes are u32-parented.** `NumberDpAttrs` and friends hang off the
//!   data point's own `id`, which is 32 bits wide; the reference attribute grouper only
//!   handles 16. Ours is generic over the width.

use anyhow::{Context, Result};
use arrow::array::{ArrayRef, BooleanArray, RecordBatch, UInt32Array};
use arrow::compute::kernels::filter::filter_record_batch;
use arrow::compute::kernels::take::take;
use arrow::datatypes::DataType;
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::otlp::metrics::MetricType;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::consts;
use rustc_hash::FxHashMap;
use std::sync::Arc;

use super::ChTable;
use super::attrs;
use super::child::ChildLists;
use super::common::{
    all_kept, bool_or_false, col, f64_or_zero, gather_index, i32_or_zero, ids_as_u64,
    list_or_empty, map_value, number_value, parent_ids_as_u64, split_quantiles, struct_field,
    struct_ids, ts_secs, u32_or_zero, u64_or_zero, utf8_or_empty,
};

pub(super) fn build(records: &OtapArrowRecords) -> Result<Vec<(ChTable, RecordBatch)>> {
    let Some(root) = records.get(ArrowPayloadType::UnivariateMetrics) else {
        return Ok(Vec::new());
    };
    if root.num_rows() == 0 {
        return Ok(Vec::new());
    }
    let identity = MetricIdentity::new(records, root)?;

    let mut out = Vec::new();
    // Gauge and sum are both carved out of NumberDataPoints.
    if let Some(dp) = records.get(ArrowPayloadType::NumberDataPoints) {
        out.extend(number_points(records, &identity, dp)?);
    }
    if let Some(dp) = records.get(ArrowPayloadType::HistogramDataPoints) {
        out.extend(histogram_points(records, &identity, dp)?);
    }
    if let Some(dp) = records.get(ArrowPayloadType::ExpHistogramDataPoints) {
        out.extend(exp_histogram_points(records, &identity, dp)?);
    }
    if let Some(dp) = records.get(ArrowPayloadType::SummaryDataPoints) {
        out.extend(summary_points(records, &identity, dp)?);
    }
    Ok(out)
}

/// The metric-grain columns, held at metric-row grain until a data-point table gathers them.
struct MetricIdentity {
    /// The first 11 columns of `METRICS_IDENTITY_COLUMNS`, in DDL order. The remaining
    /// three — `Attributes`, `StartTimeUnix`, `TimeUnix` — are data-point grain and are
    /// supplied by each caller.
    envelope: Vec<ArrayRef>,
    aggregation_temporality: ArrayRef,
    is_monotonic: ArrayRef,
    /// `metric_type` per metric row, used to split gauge from sum.
    metric_type: Vec<u8>,
    /// Metric `id` → its row in the metrics batch.
    id_to_row: FxHashMap<u64, u32>,
}

impl MetricIdentity {
    fn new(records: &OtapArrowRecords, root: &RecordBatch) -> Result<Self> {
        let len = root.num_rows();
        let resource_ids = struct_ids(
            struct_field(root, consts::RESOURCE, consts::ID).as_ref(),
            len,
        )?;
        let scope_ids = struct_ids(struct_field(root, consts::SCOPE, consts::ID).as_ref(), len)?;

        let resource_attrs =
            attrs::maps_for(records, ArrowPayloadType::ResourceAttrs, &resource_ids)?;
        let scope_attrs = attrs::maps_for(records, ArrowPayloadType::ScopeAttrs, &scope_ids)?;
        let service_name = map_value(&resource_attrs, "service.name")?;

        let envelope: Vec<ArrayRef> = vec![
            Arc::new(resource_attrs),
            utf8_or_empty(
                struct_field(root, consts::RESOURCE, consts::SCHEMA_URL).as_ref(),
                len,
            )?,
            utf8_or_empty(
                struct_field(root, consts::SCOPE, consts::NAME).as_ref(),
                len,
            )?,
            utf8_or_empty(
                struct_field(root, consts::SCOPE, consts::VERSION).as_ref(),
                len,
            )?,
            Arc::new(scope_attrs),
            u32_or_zero(
                struct_field(root, consts::SCOPE, consts::DROPPED_ATTRIBUTES_COUNT).as_ref(),
                len,
            )?,
            // As with logs, the root `schema_url` is the scope's; the resource's own lives
            // inside the resource struct.
            utf8_or_empty(col(root, consts::SCHEMA_URL), len)?,
            service_name,
            utf8_or_empty(col(root, consts::NAME), len)?,
            utf8_or_empty(col(root, consts::DESCRIPTION), len)?,
            utf8_or_empty(col(root, consts::UNIT), len)?,
        ];

        let ids = ids_as_u64(col(root, consts::ID), len)?;
        let mut id_to_row: FxHashMap<u64, u32> = FxHashMap::default();
        id_to_row.reserve(ids.len());
        for (row, id) in ids.iter().enumerate() {
            let _ = id_to_row.insert(*id, row as u32);
        }

        Ok(Self {
            envelope,
            aggregation_temporality: i32_or_zero(col(root, consts::AGGREGATION_TEMPORALITY), len)?,
            is_monotonic: bool_or_false(col(root, consts::IS_MONOTONIC), len)?,
            metric_type: super::common::u8_values(col(root, consts::METRIC_TYPE), len)?,
            id_to_row,
        })
    }

    /// Broadcast the metric-grain envelope down to data-point grain.
    fn broadcast(&self, gather: &UInt32Array) -> Result<Vec<ArrayRef>> {
        self.envelope
            .iter()
            .map(|c| Ok(take(c.as_ref(), gather, None)?))
            .collect()
    }
}

/// The per-data-point work shared by all four data-point tables: resolve the parent metric,
/// pivot the data point's own attributes, and read the two timestamps and the flags.
struct DpContext<'a> {
    batch: &'a RecordBatch,
    len: usize,
    gather: UInt32Array,
    /// `false` for data points whose `parent_id` names no metric in this batch.
    keep: BooleanArray,
    /// Data-point `id`s, the parent key for the attribute and exemplar tables.
    ids: Vec<u64>,
    /// The 14-column identity envelope at data-point grain.
    identity: Vec<ArrayRef>,
    flags: ArrayRef,
}

impl<'a> DpContext<'a> {
    fn new(
        records: &OtapArrowRecords,
        identity: &MetricIdentity,
        batch: &'a RecordBatch,
        attr_pt: ArrowPayloadType,
    ) -> Result<Self> {
        let len = batch.num_rows();
        let parents = parent_ids_as_u64(col(batch, consts::PARENT_ID), len)?;
        let (gather, keep) = gather_index(&parents, &identity.id_to_row);

        let ids = ids_as_u64(col(batch, consts::ID), len)?;
        let dp_parents: Vec<Option<u64>> = ids.iter().copied().map(Some).collect();
        let attributes = attrs::maps_for(records, attr_pt, &dp_parents)?;

        let mut envelope = identity.broadcast(&gather)?;
        envelope.push(Arc::new(attributes));
        envelope.push(ts_secs(col(batch, consts::START_TIME_UNIX_NANO), len)?);
        envelope.push(ts_secs(col(batch, consts::TIME_UNIX_NANO), len)?);

        Ok(Self {
            batch,
            len,
            gather,
            keep,
            ids,
            identity: envelope,
            flags: u32_or_zero(col(batch, consts::FLAGS), len)?,
        })
    }

    fn col(&self, name: &str) -> Option<&ArrayRef> {
        col(self.batch, name)
    }

    /// Metric-grain column, gathered to data-point grain.
    fn broadcast(&self, column: &ArrayRef) -> Result<ArrayRef> {
        Ok(take(column.as_ref(), &self.gather, None)?)
    }

    /// The five flattened `Exemplars.*` columns for this data-point table.
    fn exemplars(
        &self,
        records: &OtapArrowRecords,
        exemplar_pt: ArrowPayloadType,
        exemplar_attr_pt: ArrowPayloadType,
    ) -> Result<Vec<ArrayRef>> {
        let ex = ChildLists::new(records, exemplar_pt, exemplar_attr_pt, &self.ids)?;
        Ok(vec![
            ex.list(ex.attributes())?,
            ex.list(ex.ts_secs(consts::TIME_UNIX_NANO)?)?,
            ex.list(ex.number_value()?)?,
            ex.list(ex.hex(consts::SPAN_ID)?)?,
            ex.list(ex.hex(consts::TRACE_ID)?)?,
        ])
    }

    /// Assemble and, if some rows were excluded, filter.
    ///
    /// `extra_keep` narrows further — that is how the gauge/sum split is applied.
    fn finish(
        &self,
        table: ChTable,
        datapoint: Vec<ArrayRef>,
        extra_keep: Option<&BooleanArray>,
    ) -> Result<Option<(ChTable, RecordBatch)>> {
        let mask = match extra_keep {
            Some(extra) => arrow::compute::kernels::boolean::and(&self.keep, extra)?,
            None => self.keep.clone(),
        };
        // Nothing survives — skip building the batch at all. This is the common case for the
        // gauge/sum split, where each table rejects the other's data points.
        if mask.true_count() == 0 {
            return Ok(None);
        }

        let mut columns = self.identity.clone();
        columns.extend(datapoint);
        let batch = RecordBatch::try_new(table.schema(), columns)
            .with_context(|| format!("{table:?} columns do not match the ClickHouse schema"))?;

        Ok(Some(match all_kept(&mask) {
            true => (table, batch),
            false => (table, filter_record_batch(&batch, &mask)?),
        }))
    }

    /// A mask over data-point rows selecting those whose parent metric has `want` as its type.
    fn of_type(&self, metric_type: &[u8], want: MetricType) -> BooleanArray {
        (0..self.len)
            .map(|i| {
                let metric_row = self.gather.value(i) as usize;
                Some(metric_type.get(metric_row).copied() == Some(want as u8))
            })
            .collect()
    }
}

/// `NumberDataPoints` → the gauge table and the sum table.
fn number_points(
    records: &OtapArrowRecords,
    identity: &MetricIdentity,
    batch: &RecordBatch,
) -> Result<Vec<(ChTable, RecordBatch)>> {
    let ctx = DpContext::new(records, identity, batch, ArrowPayloadType::NumberDpAttrs)?;

    let value = number_value(
        ctx.col(consts::INT_VALUE),
        ctx.col(consts::DOUBLE_VALUE),
        ctx.len,
    )?;
    let exemplars = ctx.exemplars(
        records,
        ArrowPayloadType::NumberDpExemplars,
        ArrowPayloadType::NumberDpExemplarAttrs,
    )?;

    // Gauge and sum share these; sum then appends its two extra columns.
    let mut shared = vec![value, ctx.flags.clone()];
    shared.extend(exemplars);

    let mut out = Vec::new();
    let gauge_mask = ctx.of_type(&identity.metric_type, MetricType::Gauge);
    if let Some(entry) = ctx.finish(ChTable::MetricsGauge, shared.clone(), Some(&gauge_mask))? {
        out.push(entry);
    }

    let mut sum_columns = shared;
    sum_columns.push(ctx.broadcast(&identity.aggregation_temporality)?);
    sum_columns.push(ctx.broadcast(&identity.is_monotonic)?);
    let sum_mask = ctx.of_type(&identity.metric_type, MetricType::Sum);
    if let Some(entry) = ctx.finish(ChTable::MetricsSum, sum_columns, Some(&sum_mask))? {
        out.push(entry);
    }
    Ok(out)
}

fn histogram_points(
    records: &OtapArrowRecords,
    identity: &MetricIdentity,
    batch: &RecordBatch,
) -> Result<Vec<(ChTable, RecordBatch)>> {
    let ctx = DpContext::new(records, identity, batch, ArrowPayloadType::HistogramDpAttrs)?;

    let mut columns = vec![
        u64_or_zero(ctx.col(consts::HISTOGRAM_COUNT), ctx.len)?,
        f64_or_zero(ctx.col(consts::HISTOGRAM_SUM), ctx.len)?,
        list_or_empty(
            ctx.col(consts::HISTOGRAM_BUCKET_COUNTS),
            ctx.len,
            &DataType::UInt64,
        )?,
        list_or_empty(
            ctx.col(consts::HISTOGRAM_EXPLICIT_BOUNDS),
            ctx.len,
            &DataType::Float64,
        )?,
    ];
    columns.extend(ctx.exemplars(
        records,
        ArrowPayloadType::HistogramDpExemplars,
        ArrowPayloadType::HistogramDpExemplarAttrs,
    )?);
    columns.extend([
        ctx.flags.clone(),
        f64_or_zero(ctx.col(consts::HISTOGRAM_MIN), ctx.len)?,
        f64_or_zero(ctx.col(consts::HISTOGRAM_MAX), ctx.len)?,
        ctx.broadcast(&identity.aggregation_temporality)?,
    ]);

    Ok(ctx
        .finish(ChTable::MetricsHistogram, columns, None)?
        .into_iter()
        .collect())
}

fn exp_histogram_points(
    records: &OtapArrowRecords,
    identity: &MetricIdentity,
    batch: &RecordBatch,
) -> Result<Vec<(ChTable, RecordBatch)>> {
    let ctx = DpContext::new(
        records,
        identity,
        batch,
        ArrowPayloadType::ExpHistogramDpAttrs,
    )?;

    // `positive` / `negative` are struct columns holding an offset and a bucket-count list;
    // ClickHouse stores each member as its own column.
    let buckets = |side: &str, member: &str| struct_field(batch, side, member);

    let mut columns = vec![
        u64_or_zero(ctx.col(consts::HISTOGRAM_COUNT), ctx.len)?,
        f64_or_zero(ctx.col(consts::HISTOGRAM_SUM), ctx.len)?,
        i32_or_zero(ctx.col(consts::EXP_HISTOGRAM_SCALE), ctx.len)?,
        u64_or_zero(ctx.col(consts::EXP_HISTOGRAM_ZERO_COUNT), ctx.len)?,
        i32_or_zero(
            buckets(consts::EXP_HISTOGRAM_POSITIVE, consts::EXP_HISTOGRAM_OFFSET).as_ref(),
            ctx.len,
        )?,
        list_or_empty(
            buckets(
                consts::EXP_HISTOGRAM_POSITIVE,
                consts::EXP_HISTOGRAM_BUCKET_COUNTS,
            )
            .as_ref(),
            ctx.len,
            &DataType::UInt64,
        )?,
        i32_or_zero(
            buckets(consts::EXP_HISTOGRAM_NEGATIVE, consts::EXP_HISTOGRAM_OFFSET).as_ref(),
            ctx.len,
        )?,
        list_or_empty(
            buckets(
                consts::EXP_HISTOGRAM_NEGATIVE,
                consts::EXP_HISTOGRAM_BUCKET_COUNTS,
            )
            .as_ref(),
            ctx.len,
            &DataType::UInt64,
        )?,
    ];
    columns.extend(ctx.exemplars(
        records,
        ArrowPayloadType::ExpHistogramDpExemplars,
        ArrowPayloadType::ExpHistogramDpExemplarAttrs,
    )?);
    columns.extend([
        ctx.flags.clone(),
        f64_or_zero(ctx.col(consts::HISTOGRAM_MIN), ctx.len)?,
        f64_or_zero(ctx.col(consts::HISTOGRAM_MAX), ctx.len)?,
        ctx.broadcast(&identity.aggregation_temporality)?,
    ]);
    // OTAP's `zero_threshold` has no column in the exporter schema, so it is dropped —
    // the Go exporter drops it too.

    Ok(ctx
        .finish(ChTable::MetricsExpHistogram, columns, None)?
        .into_iter()
        .collect())
}

fn summary_points(
    records: &OtapArrowRecords,
    identity: &MetricIdentity,
    batch: &RecordBatch,
) -> Result<Vec<(ChTable, RecordBatch)>> {
    let ctx = DpContext::new(records, identity, batch, ArrowPayloadType::SummaryDpAttrs)?;

    let (quantiles, values) = split_quantiles(
        ctx.col(consts::SUMMARY_QUANTILE_VALUES),
        ctx.len,
        consts::SUMMARY_QUANTILE,
        consts::SUMMARY_VALUE,
    )?;

    // Summary is the one datapoint type with no exemplars.
    let columns = vec![
        u64_or_zero(ctx.col(consts::SUMMARY_COUNT), ctx.len)?,
        f64_or_zero(ctx.col(consts::SUMMARY_SUM), ctx.len)?,
        quantiles,
        values,
        ctx.flags.clone(),
    ];

    Ok(ctx
        .finish(ChTable::MetricsSummary, columns, None)?
        .into_iter()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::super::tests_support as support;
    use super::*;

    fn tables() -> Vec<(ChTable, RecordBatch)> {
        let mut pl = support::metrics_payload();
        super::super::to_record_batches(&mut pl).unwrap()
    }

    fn table(t: ChTable) -> RecordBatch {
        tables()
            .into_iter()
            .find(|(k, _)| *k == t)
            .unwrap_or_else(|| panic!("no batch for {t:?}"))
            .1
    }

    #[test]
    fn every_datapoint_type_reaches_its_own_table() {
        let out = tables();
        let mut kinds: Vec<_> = out.iter().map(|(t, _)| *t).collect();
        kinds.sort_by_key(|t| format!("{t:?}"));
        assert_eq!(
            kinds,
            vec![
                ChTable::MetricsExpHistogram,
                ChTable::MetricsGauge,
                ChTable::MetricsHistogram,
                ChTable::MetricsSum,
                ChTable::MetricsSummary,
            ]
        );
        for (t, b) in out {
            assert_eq!(b.schema(), t.schema(), "{t:?} schema");
            support::assert_no_nulls(&b);
        }
    }

    /// Gauge and sum come out of one `NumberDataPoints` batch; only `metric_type` separates
    /// them, so a broken split would silently file counters as gauges.
    #[test]
    fn number_points_split_by_metric_type() {
        let gauge = table(ChTable::MetricsGauge);
        let sum = table(ChTable::MetricsSum);

        assert_eq!(gauge.num_rows(), 1);
        assert_eq!(support::str_col(&gauge, "MetricName"), vec!["cpu.util"]);
        assert_eq!(support::f64_col(&gauge, "Value"), vec![0.75]);

        assert_eq!(sum.num_rows(), 2);
        assert_eq!(
            support::str_col(&sum, "MetricName"),
            vec!["net.bytes", "net.bytes"]
        );
        // Int-valued data points still land in the Float64 column.
        assert_eq!(support::f64_col(&sum, "Value"), vec![4096.0, 8192.0]);
        // Cumulative = 2, and monotonic — both metric-level, broadcast down to each point.
        assert_eq!(support::i32_col(&sum, "AggregationTemporality"), vec![2, 2]);
        assert_eq!(support::bool_col(&sum, "IsMonotonic"), vec![true, true]);
    }

    /// The whole point of the gather: metric-level identity must reach every data point.
    #[test]
    fn metric_level_columns_are_broadcast_to_every_point() {
        let sum = table(ChTable::MetricsSum);
        assert_eq!(support::str_col(&sum, "MetricUnit"), vec!["By", "By"]);
        assert_eq!(
            support::str_col(&sum, "ServiceName"),
            vec!["patrolli", "patrolli"]
        );
        assert_eq!(support::str_col(&sum, "ScopeName").len(), 2);
        for row in 0..2 {
            assert!(
                support::map_col(&sum, "ResourceAttributes", row)
                    .contains(&("service.name".to_string(), "patrolli".to_string()))
            );
            assert_eq!(
                support::map_col(&sum, "ScopeAttributes", row),
                vec![("patrolli.node.id".to_string(), "n1".to_string())]
            );
        }
        // Data-point attributes stay per point.
        assert_eq!(
            support::map_col(&sum, "Attributes", 0),
            vec![("iface".to_string(), "eth0".to_string())]
        );
        assert_eq!(
            support::map_col(&sum, "Attributes", 1),
            vec![("iface".to_string(), "eth1".to_string())]
        );
    }

    #[test]
    fn histogram_carries_its_buckets_and_bounds() {
        let h = table(ChTable::MetricsHistogram);
        assert_eq!(h.num_rows(), 1);
        assert_eq!(support::u64_col(&h, "Count"), vec![3]);
        assert_eq!(support::f64_col(&h, "Sum"), vec![30.0]);
        assert_eq!(support::list_u64(&h, "BucketCounts", 0), vec![1, 2]);
        assert_eq!(support::list_f64(&h, "ExplicitBounds", 0), vec![10.0]);
        assert_eq!(support::f64_col(&h, "Min"), vec![5.0]);
        assert_eq!(support::f64_col(&h, "Max"), vec![20.0]);
    }

    #[test]
    fn exp_histogram_splits_the_positive_and_negative_buckets() {
        let e = table(ChTable::MetricsExpHistogram);
        assert_eq!(e.num_rows(), 1);
        assert_eq!(support::u64_col(&e, "ZeroCount"), vec![1]);
        assert_eq!(support::list_u64(&e, "PositiveBucketCounts", 0), vec![2, 2]);
        assert_eq!(support::list_u64(&e, "NegativeBucketCounts", 0), vec![1]);
        assert_eq!(support::i32_col(&e, "Scale"), vec![2]);
        assert_eq!(support::i32_col(&e, "PositiveOffset"), vec![3]);
        assert_eq!(support::i32_col(&e, "NegativeOffset"), vec![-1]);
    }

    #[test]
    fn summary_splits_quantiles_into_two_parallel_arrays() {
        let s = table(ChTable::MetricsSummary);
        assert_eq!(s.num_rows(), 1);
        assert_eq!(support::u64_col(&s, "Count"), vec![4]);
        assert_eq!(support::f64_col(&s, "Sum"), vec![400.0]);
        assert_eq!(
            support::list_f64(&s, "ValueAtQuantiles.Quantile", 0),
            vec![0.5, 0.99]
        );
        assert_eq!(
            support::list_f64(&s, "ValueAtQuantiles.Value", 0),
            vec![90.0, 190.0]
        );
    }

    #[test]
    fn exemplars_ride_along_with_their_data_point() {
        let g = table(ChTable::MetricsGauge);
        assert_eq!(support::list_f64(&g, "Exemplars.Value", 0), vec![0.75]);
        assert_eq!(
            support::list_str(&g, "Exemplars.TraceId", 0),
            vec!["88".repeat(16)]
        );
        assert_eq!(
            support::list_str(&g, "Exemplars.SpanId", 0),
            vec!["99".repeat(8)]
        );
        assert_eq!(
            support::list_maps(&g, "Exemplars.FilteredAttributes", 0),
            vec![vec![("ex.key".to_string(), "ex.val".to_string())]]
        );

        // Sum's data points have no exemplars, so they get empty lists rather than nulls.
        let s = table(ChTable::MetricsSum);
        assert!(support::list_f64(&s, "Exemplars.Value", 0).is_empty());
    }

    #[test]
    fn timestamps_narrow_to_second_precision() {
        let g = table(ChTable::MetricsGauge);
        assert_eq!(support::ts_secs_col(&g, "TimeUnix"), vec![1_700_000_060]);
        assert_eq!(
            support::ts_secs_col(&g, "StartTimeUnix"),
            vec![1_700_000_000]
        );
    }

    #[test]
    fn survives_a_transport_encoded_round_trip() {
        let direct = tables();
        let mut round_tripped = support::round_trip(&support::metrics_payload());
        let after = super::super::to_record_batches(&mut round_tripped).unwrap();

        assert_eq!(after.len(), direct.len());
        for (table, batch) in &direct {
            let other = after
                .iter()
                .find(|(t, _)| t == table)
                .unwrap_or_else(|| panic!("missing {table:?} after round trip"));
            support::assert_equivalent(&other.1, batch);
        }
    }

    #[test]
    fn empty_payload_yields_nothing() {
        let mut pl = support::empty_metrics_payload();
        assert!(super::super::to_record_batches(&mut pl).unwrap().is_empty());
    }

    /// A payload carrying only gauges must not emit an empty sum batch (and vice versa) —
    /// both come from the same `NumberDataPoints` table, so the split has to be able to
    /// produce nothing at all for one side.
    #[test]
    fn a_table_with_no_surviving_points_is_omitted() {
        let mut pl = support::gauge_only_payload();
        let out = super::super::to_record_batches(&mut pl).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, ChTable::MetricsGauge);
        assert_eq!(out[0].1.num_rows(), 1);
    }

    /// No batch this module returns is ever empty — an empty insert is pointless work and
    /// the module documents that it omits them.
    #[test]
    fn no_returned_batch_is_empty() {
        for (table, batch) in tables() {
            assert!(batch.num_rows() > 0, "{table:?} returned an empty batch");
        }
    }
}
