// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::AttrsBuilder;
use super::builder::BaseBuilder;
use arrow::datatypes::UInt32Type;
use chrono::{DateTime, Utc};
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::encode::record::metrics::*;
use otel_arrow_dfe_pdata::otap::Metrics;
use otel_arrow_dfe_pdata::otlp::metrics::MetricType;
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::AggregationTemporality;
use std::borrow::Cow;
use std::fmt::Write;
use std::net::IpAddr;

pub struct MetricRow<'a> {
    pub name: Cow<'a, str>,
    pub r#type: MetricType,
    pub description: Option<Cow<'a, str>>,
    pub unit: Option<Cow<'a, str>>,
    pub source_ip: Option<IpAddr>,
    pub source_port: Option<u16>,
    pub data_points: DataPoints<'a>,
    pub attrs: &'a [(super::ZString<'a>, super::Value<'a>)],
}

#[derive(Clone)]
pub struct MetricRowOwned {
    pub name: super::ZStringOwned,
    pub r#type: MetricType,
    pub description: Option<super::ZStringOwned>,
    pub unit: Option<super::ZStringOwned>,
    pub source_ip: Option<IpAddr>,
    pub source_port: Option<u16>,
    pub data_points: DataPointsOwned,
    pub attrs: Vec<(super::ZStringOwned, super::ValueOwned)>,
}

pub enum DataPoints<'a> {
    Sum(Sum<'a>),
    Gauge(Gauge<'a>),
    Summary(Summary<'a>),
    Histogram(Histogram<'a>),
    ExponentialHistogram(ExponentialHistogram<'a>),
}

#[derive(Clone)]
pub enum DataPointsOwned {
    Sum(SumOwned),
    Gauge(GaugeOwned),
    Summary(SummaryOwned),
    Histogram(HistogramOwned),
    ExponentialHistogram(ExponentialHistogramOwned),
}

pub struct Exemplar<'a> {
    pub time: DateTime<Utc>,
    pub value: NumberValue,
    pub span_id: Option<[u8; 8]>,
    pub trace_id: Option<[u8; 16]>,
    pub attrs: &'a [(super::ZString<'a>, super::Value<'a>)],
}

#[derive(Clone)]
pub struct ExemplarOwned {
    pub time: DateTime<Utc>,
    pub value: NumberValue,
    pub span_id: Option<[u8; 8]>,
    pub trace_id: Option<[u8; 16]>,
    pub attrs: Vec<(super::ZStringOwned, super::ValueOwned)>,
}

pub struct Sum<'a> {
    pub data_points: &'a [NumberDataPoint<'a>],
    pub aggregation_temporality: AggregationTemporality,
    pub is_monotonic: bool,
}

#[derive(Clone)]
pub struct SumOwned {
    pub data_points: Vec<NumberDataPointOwned>,
    pub aggregation_temporality: AggregationTemporality,
    pub is_monotonic: bool,
}

pub struct Gauge<'a> {
    pub data_points: &'a [NumberDataPoint<'a>],
}

#[derive(Clone)]
pub struct GaugeOwned {
    pub data_points: Vec<NumberDataPointOwned>,
}

pub struct Summary<'a> {
    pub data_points: &'a [SummaryDataPoint<'a>],
}

#[derive(Clone)]
pub struct SummaryOwned {
    pub data_points: Vec<SummaryDataPointOwned>,
}

pub struct Histogram<'a> {
    pub data_points: &'a [HistogramDataPoint<'a>],
    pub aggregation_temporality: AggregationTemporality,
}

#[derive(Clone)]
pub struct HistogramOwned {
    pub data_points: Vec<HistogramDataPointOwned>,
    pub aggregation_temporality: AggregationTemporality,
}

pub struct ExponentialHistogram<'a> {
    pub data_points: &'a [ExponentialHistogramDataPoint<'a>],
    pub aggregation_temporality: AggregationTemporality,
}

#[derive(Clone)]
pub struct ExponentialHistogramOwned {
    pub data_points: Vec<ExponentialHistogramDataPointOwned>,
    pub aggregation_temporality: AggregationTemporality,
}

#[derive(Clone, Copy)]
pub enum NumberValue {
    Float(f64),
    Int(i64),
}

pub struct NumberDataPoint<'a> {
    pub start_time: Option<DateTime<Utc>>,
    pub time: DateTime<Utc>,
    pub value: NumberValue,
    pub flags: u32,
    pub attrs: &'a [(super::ZString<'a>, super::Value<'a>)],
    pub exemplars: &'a [Exemplar<'a>],
}

#[derive(Clone)]
pub struct NumberDataPointOwned {
    pub start_time: Option<DateTime<Utc>>,
    pub time: DateTime<Utc>,
    pub value: NumberValue,
    pub flags: u32,
    pub attrs: Vec<(super::ZStringOwned, super::ValueOwned)>,
    pub exemplars: Vec<ExemplarOwned>,
}

#[derive(Clone, Copy)]
pub struct Quantile {
    pub quantile: f64,
    pub value: f64,
}

pub struct SummaryDataPoint<'a> {
    pub start_time: Option<DateTime<Utc>>,
    pub time: DateTime<Utc>,
    pub count: u64,
    pub sum: f64,
    pub quantiles: &'a [Quantile],
    pub flags: u32,
    pub attrs: &'a [(super::ZString<'a>, super::Value<'a>)],
}

#[derive(Clone)]
pub struct SummaryDataPointOwned {
    pub start_time: Option<DateTime<Utc>>,
    pub time: DateTime<Utc>,
    pub count: u64,
    pub sum: f64,
    pub quantiles: Vec<Quantile>,
    pub flags: u32,
    pub attrs: Vec<(super::ZStringOwned, super::ValueOwned)>,
}

pub struct HistogramDataPoint<'a> {
    pub start_time: Option<DateTime<Utc>>,
    pub time: DateTime<Utc>,
    pub count: u64,
    pub sum: Option<f64>,
    pub bucket_counts: &'a [u64],
    pub explicit_bounds: &'a [f64],
    pub flags: u32,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub attrs: &'a [(super::ZString<'a>, super::Value<'a>)],
    pub exemplars: &'a [Exemplar<'a>],
}

#[derive(Clone)]
pub struct HistogramDataPointOwned {
    pub start_time: Option<DateTime<Utc>>,
    pub time: DateTime<Utc>,
    pub count: u64,
    pub sum: Option<f64>,
    pub bucket_counts: Vec<u64>,
    pub explicit_bounds: Vec<f64>,
    pub flags: u32,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub attrs: Vec<(super::ZStringOwned, super::ValueOwned)>,
    pub exemplars: Vec<ExemplarOwned>,
}

/// One side (positive or negative) of an exponential histogram's buckets.
/// The whole message is optional in OTLP, so callers wrap this in `Option`.
/// `offset` is a plain `sint32` (0 is meaningful) and `bucket_counts` may be empty.
pub struct Buckets<'a> {
    pub offset: i32,
    pub bucket_counts: &'a [u64],
}

#[derive(Clone)]
pub struct BucketsOwned {
    pub offset: i32,
    pub bucket_counts: Vec<u64>,
}

pub struct ExponentialHistogramDataPoint<'a> {
    pub start_time: Option<DateTime<Utc>>,
    pub time: DateTime<Utc>,
    pub count: u64,
    pub sum: Option<f64>,
    pub scale: i32,
    pub zero_count: u64,
    pub positive: Option<Buckets<'a>>,
    pub negative: Option<Buckets<'a>>,
    pub zero_threshold: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub flags: u32,
    pub attrs: &'a [(super::ZString<'a>, super::Value<'a>)],
    pub exemplars: &'a [Exemplar<'a>],
}

#[derive(Clone)]
pub struct ExponentialHistogramDataPointOwned {
    pub start_time: Option<DateTime<Utc>>,
    pub time: DateTime<Utc>,
    pub count: u64,
    pub sum: Option<f64>,
    pub scale: i32,
    pub zero_count: u64,
    pub positive: Option<BucketsOwned>,
    pub negative: Option<BucketsOwned>,
    pub zero_threshold: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub flags: u32,
    pub attrs: Vec<(super::ZStringOwned, super::ValueOwned)>,
    pub exemplars: Vec<ExemplarOwned>,
}

pub struct MetricsBuilder {
    base: BaseBuilder,
    rows: MetricsRecordBatchBuilder,

    number_dp: NumberDataPointsRecordBatchBuilder,
    summary_dp: SummaryDataPointsRecordBatchBuilder,
    histogram_dp: HistogramDataPointsRecordBatchBuilder,
    exp_histogram_dp: ExponentialHistogramDataPointsRecordBatchBuilder,

    number_dp_attr: AttrsBuilder<UInt32Type>,
    summary_dp_attr: AttrsBuilder<UInt32Type>,
    histogram_dp_attr: AttrsBuilder<UInt32Type>,
    exp_histogram_dp_attr: AttrsBuilder<UInt32Type>,

    number_dp_exemplars: ExemplarsRecordBatchBuilder,
    histogram_dp_exemplars: ExemplarsRecordBatchBuilder,
    exp_histogram_dp_exemplars: ExemplarsRecordBatchBuilder,

    number_dp_exemplars_attr: AttrsBuilder<UInt32Type>,
    histogram_dp_exemplars_attr: AttrsBuilder<UInt32Type>,
    exp_histogram_dp_exemplars_attr: AttrsBuilder<UInt32Type>,

    // Data-point and exemplar ids are explicit per-table `u32` counters that must
    // persist across `push_row` calls (each metric row appends more data points to
    // the shared builders). They are reset to 0 in `finish`.
    number_dp_id: u32,
    summary_dp_id: u32,
    histogram_dp_id: u32,
    exp_histogram_dp_id: u32,
    number_dp_exemplar_id: u32,
    histogram_dp_exemplar_id: u32,
    exp_histogram_dp_exemplar_id: u32,
}

impl Default for MetricsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsBuilder {
    pub fn new() -> Self {
        Self {
            base: Default::default(),
            rows: MetricsRecordBatchBuilder::new(),

            number_dp: NumberDataPointsRecordBatchBuilder::new(),
            summary_dp: SummaryDataPointsRecordBatchBuilder::new(),
            histogram_dp: HistogramDataPointsRecordBatchBuilder::new(),
            exp_histogram_dp: ExponentialHistogramDataPointsRecordBatchBuilder::new(),

            number_dp_attr: AttrsBuilder::new(),
            summary_dp_attr: AttrsBuilder::new(),
            histogram_dp_attr: AttrsBuilder::new(),
            exp_histogram_dp_attr: AttrsBuilder::new(),

            number_dp_exemplars: ExemplarsRecordBatchBuilder::new(),
            histogram_dp_exemplars: ExemplarsRecordBatchBuilder::new(),
            exp_histogram_dp_exemplars: ExemplarsRecordBatchBuilder::new(),

            number_dp_exemplars_attr: AttrsBuilder::new(),
            histogram_dp_exemplars_attr: AttrsBuilder::new(),
            exp_histogram_dp_exemplars_attr: AttrsBuilder::new(),

            number_dp_id: 0,
            summary_dp_id: 0,
            histogram_dp_id: 0,
            exp_histogram_dp_id: 0,
            number_dp_exemplar_id: 0,
            histogram_dp_exemplar_id: 0,
            exp_histogram_dp_exemplar_id: 0,
        }
    }

    /// Replaces every OTAP sub-builder with a fresh one. Called at the end of
    /// [`Self::finish`], because `finish` on those builders only drains the columns
    /// that hold values — see `BaseBuilder::finish` for the full explanation and why
    /// replacing costs nothing. On this side the state that survives a `finish` is:
    ///
    ///   - `rows.is_monotonic`: an adaptive boolean, which for an all-null/all-false
    ///     column produces no array *and* keeps the nulls it counted. Gauge and Summary
    ///     rows pass `None`, so a monitor emitting gauges for hours and then one
    ///     monotonic Sum back-fills every one of them and trips the length check —
    ///     which upstream turns into an unconditional `panic!` (encode/record/
    ///     metrics.rs:261 `fn check`), and a panic takes the process with it
    ///     (`common::panic`).
    ///   - `rows.description`/`rows.unit`: fed `b""` when an item has neither, which is
    ///     the column default and so counts as "still unused" — a bit per metric row.
    ///   - `rows.aggregation_temporality`, `rows.scope_schema_url`, `number_dp.flags`:
    ///     same, via nulls or default values.
    ///
    /// otap-df-pdata v0.50.0, encode/record/array/boolean.rs:115 and array.rs:660; both
    /// still present in otel-arrow-dfe-pdata v0.54.1.
    fn reset_builders(&mut self) {
        self.rows = MetricsRecordBatchBuilder::new();

        self.number_dp = NumberDataPointsRecordBatchBuilder::new();
        self.summary_dp = SummaryDataPointsRecordBatchBuilder::new();
        self.histogram_dp = HistogramDataPointsRecordBatchBuilder::new();
        self.exp_histogram_dp = ExponentialHistogramDataPointsRecordBatchBuilder::new();

        self.number_dp_exemplars = ExemplarsRecordBatchBuilder::new();
        self.histogram_dp_exemplars = ExemplarsRecordBatchBuilder::new();
        self.exp_histogram_dp_exemplars = ExemplarsRecordBatchBuilder::new();
    }

    pub fn scope(&mut self, scope: super::Scope) {
        self.base.scope(scope);
    }

    pub fn resource(&mut self, resource: super::Resource) {
        self.base.resource(resource);
    }

    /// Metric rows appended so far — *not* the data-point count. Bounded by
    /// `u16::MAX` because data points reference their metric by a `u16`
    /// parent id.
    pub fn num_rows(&self) -> usize {
        self.base.num_rows()
    }

    /// Data points appended so far, across all four data-point tables. This is
    /// the count [`Payload::num_items`](super::Payload::num_items) will report
    /// for the finished payload, so it is what `batch_size` should be compared
    /// against. It is `>=` [`Self::num_rows`], and `0` for rows carrying no data
    /// points.
    pub fn num_items(&self) -> usize {
        (self.number_dp_id + self.summary_dp_id + self.histogram_dp_id + self.exp_histogram_dp_id)
            as usize
    }

    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
    }

    pub fn push_row(&mut self, row: MetricRow) {
        let (agg, mono) = aggregation_of(&row.data_points);
        let id = self.append_metric_fields(
            row.name.as_ref(),
            row.r#type,
            row.description.as_deref(),
            row.unit.as_deref(),
            row.source_ip,
            row.source_port,
            agg,
            mono,
        );
        for (k, v) in row.attrs {
            self.base.append_row_attr(id, k.as_ref(), v);
        }
        match &row.data_points {
            DataPoints::Sum(s) => {
                for dp in s.data_points {
                    self.append_number_dp(id, dp);
                }
            }
            DataPoints::Gauge(g) => {
                for dp in g.data_points {
                    self.append_number_dp(id, dp);
                }
            }
            DataPoints::Summary(s) => {
                for dp in s.data_points {
                    self.append_summary_dp(id, dp);
                }
            }
            DataPoints::Histogram(h) => {
                for dp in h.data_points {
                    self.append_histogram_dp(id, dp);
                }
            }
            DataPoints::ExponentialHistogram(e) => {
                for dp in e.data_points {
                    self.append_exp_histogram_dp(id, dp);
                }
            }
        }
    }

    pub fn push_row_owned(&mut self, row: MetricRowOwned) {
        let (agg, mono) = aggregation_of_owned(&row.data_points);
        let id = self.append_metric_fields(
            row.name.as_ref(),
            row.r#type,
            row.description.as_ref().map(|d| d.as_ref()),
            row.unit.as_ref().map(|u| u.as_ref()),
            row.source_ip,
            row.source_port,
            agg,
            mono,
        );
        for (k, v) in &row.attrs {
            self.base.append_row_attr(id, k.as_ref(), &v.as_ref_z());
        }
        // The owned data points are re-viewed as borrowed ones so they can reuse
        // the append helpers of `push_row`. The views carry no attributes or
        // exemplars: those are appended straight from the owned slices against the
        // id the helper returns, so no borrowed copy of them is ever collected.
        match &row.data_points {
            DataPointsOwned::Sum(s) => self.append_number_dps_owned(id, &s.data_points),
            DataPointsOwned::Gauge(g) => self.append_number_dps_owned(id, &g.data_points),
            DataPointsOwned::Summary(s) => {
                for dp in &s.data_points {
                    let dp_id = self.append_summary_dp(
                        id,
                        &SummaryDataPoint {
                            start_time: dp.start_time,
                            time: dp.time,
                            count: dp.count,
                            sum: dp.sum,
                            quantiles: &dp.quantiles,
                            flags: dp.flags,
                            attrs: &[],
                        },
                    );
                    append_owned_attrs(&mut self.summary_dp_attr, dp_id, &dp.attrs);
                }
            }
            DataPointsOwned::Histogram(h) => self.append_histogram_dps_owned(id, &h.data_points),
            DataPointsOwned::ExponentialHistogram(e) => {
                self.append_exp_histogram_dps_owned(id, &e.data_points)
            }
        }
    }

    fn append_number_dps_owned(&mut self, metric_id: u16, dps: &[NumberDataPointOwned]) {
        for dp in dps {
            let dp_id = self.append_number_dp(
                metric_id,
                &NumberDataPoint {
                    start_time: dp.start_time,
                    time: dp.time,
                    value: dp.value,
                    flags: dp.flags,
                    attrs: &[],
                    exemplars: &[],
                },
            );
            append_owned_attrs(&mut self.number_dp_attr, dp_id, &dp.attrs);
            append_owned_exemplars(
                &mut self.number_dp_exemplars,
                &mut self.number_dp_exemplars_attr,
                &mut self.number_dp_exemplar_id,
                dp_id,
                &dp.exemplars,
            );
        }
    }

    fn append_histogram_dps_owned(&mut self, metric_id: u16, dps: &[HistogramDataPointOwned]) {
        for dp in dps {
            let dp_id = self.append_histogram_dp(
                metric_id,
                &HistogramDataPoint {
                    start_time: dp.start_time,
                    time: dp.time,
                    count: dp.count,
                    sum: dp.sum,
                    bucket_counts: &dp.bucket_counts,
                    explicit_bounds: &dp.explicit_bounds,
                    flags: dp.flags,
                    min: dp.min,
                    max: dp.max,
                    attrs: &[],
                    exemplars: &[],
                },
            );
            append_owned_attrs(&mut self.histogram_dp_attr, dp_id, &dp.attrs);
            append_owned_exemplars(
                &mut self.histogram_dp_exemplars,
                &mut self.histogram_dp_exemplars_attr,
                &mut self.histogram_dp_exemplar_id,
                dp_id,
                &dp.exemplars,
            );
        }
    }

    fn append_exp_histogram_dps_owned(
        &mut self,
        metric_id: u16,
        dps: &[ExponentialHistogramDataPointOwned],
    ) {
        for dp in dps {
            let positive = dp.positive.as_ref().map(|b| Buckets {
                offset: b.offset,
                bucket_counts: &b.bucket_counts,
            });
            let negative = dp.negative.as_ref().map(|b| Buckets {
                offset: b.offset,
                bucket_counts: &b.bucket_counts,
            });
            let dp_id = self.append_exp_histogram_dp(
                metric_id,
                &ExponentialHistogramDataPoint {
                    start_time: dp.start_time,
                    time: dp.time,
                    count: dp.count,
                    sum: dp.sum,
                    scale: dp.scale,
                    zero_count: dp.zero_count,
                    positive,
                    negative,
                    zero_threshold: dp.zero_threshold,
                    min: dp.min,
                    max: dp.max,
                    flags: dp.flags,
                    attrs: &[],
                    exemplars: &[],
                },
            );
            append_owned_attrs(&mut self.exp_histogram_dp_attr, dp_id, &dp.attrs);
            append_owned_exemplars(
                &mut self.exp_histogram_dp_exemplars,
                &mut self.exp_histogram_dp_exemplars_attr,
                &mut self.exp_histogram_dp_exemplar_id,
                dp_id,
                &dp.exemplars,
            );
        }
    }

    /// Appends the metric-level columns (id, resource/scope, type, name,
    /// description, unit, aggregation temporality / monotonicity) plus the
    /// source ip/port attributes for one metric, returning the metric id that
    /// data points and user attributes should be parented to.
    #[allow(clippy::too_many_arguments)]
    fn append_metric_fields(
        &mut self,
        name: &str,
        ty: MetricType,
        description: Option<&str>,
        unit: Option<&str>,
        source_ip: Option<IpAddr>,
        source_port: Option<u16>,
        aggregation_temporality: Option<i32>,
        is_monotonic: Option<bool>,
    ) -> u16 {
        let id = self.base.next_row_id();
        self.rows.append_id(id);
        self.rows.resource.append_id(Some(self.base.resource_id()));
        self.rows
            .scope
            .append_name(Some(self.base.qualified_scope_name().as_bytes()));
        self.rows
            .scope
            .append_version(Some(env!("CARGO_PKG_VERSION").as_bytes()));
        self.rows.scope.append_id(Some(self.base.scope_id()));
        self.rows.append_metric_type(ty as u8);
        self.rows.append_name(name.as_bytes());
        self.rows
            .append_description(description.unwrap_or_default().as_bytes());
        self.rows.append_unit(unit.unwrap_or_default().as_bytes());
        self.rows
            .append_aggregation_temporality(aggregation_temporality);
        self.rows.append_is_monotonic(is_monotonic);

        if let Some(source_ip) = source_ip {
            let mut ip_buf = arrayvec::ArrayString::<45>::new();
            write!(&mut ip_buf, "{}", source_ip).unwrap();
            self.base.append_row_attr(
                id,
                super::RecordAttrKey::SourceAddress.as_str(),
                &super::Value::String(super::ZString::from_ref(ip_buf.as_str())),
            );
        }
        if let Some(source_port) = source_port {
            self.base.append_row_attr(
                id,
                super::RecordAttrKey::SourcePort.as_str(),
                &super::Value::Int64(source_port as i64),
            );
        }
        id
    }

    /// Returns the data point's id, which its attributes and exemplars are parented to.
    fn append_number_dp(&mut self, metric_id: u16, dp: &NumberDataPoint) -> u32 {
        let dp_id = self.number_dp_id;
        self.number_dp_id += 1;
        self.number_dp.append_id(dp_id);
        self.number_dp.append_parent_id(metric_id);
        self.number_dp
            .append_start_time_unix_nano(dp.start_time.map(to_nanos));
        self.number_dp.append_time_unix_nano(to_nanos(dp.time));
        match dp.value {
            NumberValue::Int(v) => {
                self.number_dp.append_int_value(Some(v));
                self.number_dp.append_double_value(None);
            }
            NumberValue::Float(v) => {
                self.number_dp.append_double_value(Some(v));
                self.number_dp.append_int_value(None);
            }
        }
        self.number_dp.append_flags(dp.flags);
        for (k, v) in dp.attrs {
            v.append_to(&mut self.number_dp_attr, dp_id, k.as_ref());
        }
        for ex in dp.exemplars {
            append_exemplar(
                &mut self.number_dp_exemplars,
                &mut self.number_dp_exemplars_attr,
                &mut self.number_dp_exemplar_id,
                dp_id,
                ex,
            );
        }
        dp_id
    }

    /// Returns the data point's id, which its attributes and exemplars are parented to.
    fn append_summary_dp(&mut self, metric_id: u16, dp: &SummaryDataPoint) -> u32 {
        let dp_id = self.summary_dp_id;
        self.summary_dp_id += 1;
        self.summary_dp.append_id(dp_id);
        self.summary_dp.append_parent_id(metric_id);
        self.summary_dp
            .append_start_time_unix_nano(dp.start_time.map(to_nanos).unwrap_or_default());
        self.summary_dp.append_time_unix_nano(to_nanos(dp.time));
        self.summary_dp.append_count(dp.count);
        self.summary_dp.append_sum(dp.sum);
        self.summary_dp
            .quantile_values
            .append(dp.quantiles.iter().map(|q| (q.quantile, q.value)));
        self.summary_dp.append_flags(dp.flags);
        for (k, v) in dp.attrs {
            v.append_to(&mut self.summary_dp_attr, dp_id, k.as_ref());
        }
        dp_id
    }

    /// Returns the data point's id, which its attributes and exemplars are parented to.
    fn append_histogram_dp(&mut self, metric_id: u16, dp: &HistogramDataPoint) -> u32 {
        let dp_id = self.histogram_dp_id;
        self.histogram_dp_id += 1;
        self.histogram_dp.append_id(dp_id);
        self.histogram_dp.append_parent_id(metric_id);
        self.histogram_dp
            .append_start_time_unix_nano(dp.start_time.map(to_nanos).unwrap_or_default());
        self.histogram_dp.append_time_unix_nano(to_nanos(dp.time));
        self.histogram_dp.append_count(dp.count);
        self.histogram_dp
            .append_bucket_counts(dp.bucket_counts.iter().copied());
        self.histogram_dp
            .append_explicit_bounds(dp.explicit_bounds.iter().copied());
        self.histogram_dp.append_sum(dp.sum);
        self.histogram_dp.append_flags(dp.flags);
        self.histogram_dp.append_min(dp.min);
        self.histogram_dp.append_max(dp.max);
        for (k, v) in dp.attrs {
            v.append_to(&mut self.histogram_dp_attr, dp_id, k.as_ref());
        }
        for ex in dp.exemplars {
            append_exemplar(
                &mut self.histogram_dp_exemplars,
                &mut self.histogram_dp_exemplars_attr,
                &mut self.histogram_dp_exemplar_id,
                dp_id,
                ex,
            );
        }
        dp_id
    }

    /// Returns the data point's id, which its attributes and exemplars are parented to.
    fn append_exp_histogram_dp(
        &mut self,
        metric_id: u16,
        dp: &ExponentialHistogramDataPoint,
    ) -> u32 {
        let dp_id = self.exp_histogram_dp_id;
        self.exp_histogram_dp_id += 1;
        self.exp_histogram_dp.append_id(dp_id);
        self.exp_histogram_dp.append_parent_id(metric_id);
        self.exp_histogram_dp
            .append_start_time_unix_nano(dp.start_time.map(to_nanos).unwrap_or_default());
        self.exp_histogram_dp
            .append_time_unix_nano(to_nanos(dp.time));
        self.exp_histogram_dp.append_count(dp.count);
        self.exp_histogram_dp.append_sum(dp.sum);
        self.exp_histogram_dp.append_scale(dp.scale);
        self.exp_histogram_dp.append_zero_count(dp.zero_count);
        self.exp_histogram_dp.positive.append(
            dp.positive
                .as_ref()
                .map(|b| (b.offset, b.bucket_counts.iter().copied())),
        );
        self.exp_histogram_dp.negative.append(
            dp.negative
                .as_ref()
                .map(|b| (b.offset, b.bucket_counts.iter().copied())),
        );
        self.exp_histogram_dp.append_flags(dp.flags);
        self.exp_histogram_dp.append_min(dp.min);
        self.exp_histogram_dp.append_max(dp.max);
        self.exp_histogram_dp
            .append_zero_threshold(dp.zero_threshold);
        for (k, v) in dp.attrs {
            v.append_to(&mut self.exp_histogram_dp_attr, dp_id, k.as_ref());
        }
        for ex in dp.exemplars {
            append_exemplar(
                &mut self.exp_histogram_dp_exemplars,
                &mut self.exp_histogram_dp_exemplars_attr,
                &mut self.exp_histogram_dp_exemplar_id,
                dp_id,
                ex,
            );
        }
        dp_id
    }

    pub fn finish(&mut self) -> super::Payload {
        let mut otap_batch = OtapArrowRecords::Metrics(Metrics::default());
        // Like the logs builder, the OTAP metrics builder can't finish a zero-row
        // batch (its nested resource/scope struct columns have no length to infer),
        // so an empty builder yields an empty `Metrics` payload directly. Sub-builders
        // are drained below regardless so a reused builder starts clean.
        if self.num_rows() > 0 {
            let metrics = self.rows.finish().unwrap();
            let base = self.base.finish();

            let number_dp = self.number_dp.finish().unwrap();
            let number_dp_attr = self.number_dp_attr.finish().unwrap();
            let number_dp_exemplars = self.number_dp_exemplars.finish().unwrap();
            let number_dp_exemplars_attr = self.number_dp_exemplars_attr.finish().unwrap();

            let summary_dp = self.summary_dp.finish().unwrap();
            let summary_dp_attr = self.summary_dp_attr.finish().unwrap();

            let histogram_dp = self.histogram_dp.finish().unwrap();
            let histogram_dp_attr = self.histogram_dp_attr.finish().unwrap();
            let histogram_dp_exemplars = self.histogram_dp_exemplars.finish().unwrap();
            let histogram_dp_exemplars_attr = self.histogram_dp_exemplars_attr.finish().unwrap();

            let exp_histogram_dp = self.exp_histogram_dp.finish().unwrap();
            let exp_histogram_dp_attr = self.exp_histogram_dp_attr.finish().unwrap();
            let exp_histogram_dp_exemplars = self.exp_histogram_dp_exemplars.finish().unwrap();
            let exp_histogram_dp_exemplars_attr =
                self.exp_histogram_dp_exemplars_attr.finish().unwrap();

            self.number_dp_id = 0;
            self.summary_dp_id = 0;
            self.histogram_dp_id = 0;
            self.exp_histogram_dp_id = 0;
            self.number_dp_exemplar_id = 0;
            self.histogram_dp_exemplar_id = 0;
            self.exp_histogram_dp_exemplar_id = 0;
            // Must stay inside the `num_rows() > 0` guard: on the zero-row path nothing
            // is drained, and scope attributes appended by `BaseBuilder::scope` (which
            // does not bump the row id) have to survive to the next batch.
            self.reset_builders();

            // `UnivariateMetrics` is the root payload; every other batch is only
            // attached when it carries rows (mirrors `encode_metrics_otap_batch`).
            if metrics.num_rows() > 0 {
                otap_batch
                    .set(ArrowPayloadType::UnivariateMetrics, metrics)
                    .unwrap();
                for (payload_type, batch) in [
                    (ArrowPayloadType::MetricAttrs, base.row_attr),
                    (ArrowPayloadType::ResourceAttrs, base.resource_attr),
                    (ArrowPayloadType::ScopeAttrs, base.scope_attr),
                    (ArrowPayloadType::NumberDataPoints, number_dp),
                    (ArrowPayloadType::NumberDpAttrs, number_dp_attr),
                    (ArrowPayloadType::NumberDpExemplars, number_dp_exemplars),
                    (
                        ArrowPayloadType::NumberDpExemplarAttrs,
                        number_dp_exemplars_attr,
                    ),
                    (ArrowPayloadType::SummaryDataPoints, summary_dp),
                    (ArrowPayloadType::SummaryDpAttrs, summary_dp_attr),
                    (ArrowPayloadType::HistogramDataPoints, histogram_dp),
                    (ArrowPayloadType::HistogramDpAttrs, histogram_dp_attr),
                    (
                        ArrowPayloadType::HistogramDpExemplars,
                        histogram_dp_exemplars,
                    ),
                    (
                        ArrowPayloadType::HistogramDpExemplarAttrs,
                        histogram_dp_exemplars_attr,
                    ),
                    (ArrowPayloadType::ExpHistogramDataPoints, exp_histogram_dp),
                    (ArrowPayloadType::ExpHistogramDpAttrs, exp_histogram_dp_attr),
                    (
                        ArrowPayloadType::ExpHistogramDpExemplars,
                        exp_histogram_dp_exemplars,
                    ),
                    (
                        ArrowPayloadType::ExpHistogramDpExemplarAttrs,
                        exp_histogram_dp_exemplars_attr,
                    ),
                ] {
                    if batch.num_rows() > 0 {
                        otap_batch.set(payload_type, batch).unwrap();
                    }
                }
            }
        }
        super::Payload { data: otap_batch }
    }
}

fn to_nanos(t: DateTime<Utc>) -> i64 {
    t.timestamp_nanos_opt().unwrap_or_default()
}

/// Returns `(aggregation_temporality, is_monotonic)` for the metric row, which
/// in OTLP live on the Sum/Histogram/ExponentialHistogram wrappers (Gauge and
/// Summary carry neither).
fn aggregation_of(data_points: &DataPoints) -> (Option<i32>, Option<bool>) {
    match data_points {
        DataPoints::Sum(s) => (Some(s.aggregation_temporality as i32), Some(s.is_monotonic)),
        DataPoints::Histogram(h) => (Some(h.aggregation_temporality as i32), None),
        DataPoints::ExponentialHistogram(e) => (Some(e.aggregation_temporality as i32), None),
        DataPoints::Gauge(_) | DataPoints::Summary(_) => (None, None),
    }
}

fn aggregation_of_owned(data_points: &DataPointsOwned) -> (Option<i32>, Option<bool>) {
    match data_points {
        DataPointsOwned::Sum(s) => (Some(s.aggregation_temporality as i32), Some(s.is_monotonic)),
        DataPointsOwned::Histogram(h) => (Some(h.aggregation_temporality as i32), None),
        DataPointsOwned::ExponentialHistogram(e) => (Some(e.aggregation_temporality as i32), None),
        DataPointsOwned::Gauge(_) | DataPointsOwned::Summary(_) => (None, None),
    }
}

/// Appends owned attributes as they are, without first collecting a borrowed
/// `(ZString, Value)` copy of them.
fn append_owned_attrs(
    batch: &mut AttrsBuilder<UInt32Type>,
    parent_id: u32,
    attrs: &[(super::ZStringOwned, super::ValueOwned)],
) {
    for (k, v) in attrs {
        v.as_ref_z().append_to(batch, parent_id, k.as_ref());
    }
}

/// The owned counterpart of the exemplar loop in the borrowed `append_*_dp`
/// helpers. Free function for the same reason as [`append_exemplar`].
fn append_owned_exemplars(
    exemplars: &mut ExemplarsRecordBatchBuilder,
    exemplars_attr: &mut AttrsBuilder<UInt32Type>,
    next_id: &mut u32,
    parent_dp_id: u32,
    owned: &[ExemplarOwned],
) {
    for e in owned {
        let ex_id = append_exemplar(
            exemplars,
            exemplars_attr,
            next_id,
            parent_dp_id,
            &Exemplar {
                time: e.time,
                value: e.value,
                span_id: e.span_id,
                trace_id: e.trace_id,
                attrs: &[],
            },
        );
        append_owned_attrs(exemplars_attr, ex_id, &e.attrs);
    }
}

/// Appends one exemplar plus its attributes and returns its id. Free function
/// (rather than a method) so the caller can pass disjoint `&mut` borrows of its
/// exemplar builder fields.
fn append_exemplar(
    exemplars: &mut ExemplarsRecordBatchBuilder,
    exemplars_attr: &mut AttrsBuilder<UInt32Type>,
    next_id: &mut u32,
    parent_dp_id: u32,
    ex: &Exemplar,
) -> u32 {
    let ex_id = *next_id;
    *next_id += 1;
    exemplars.append_id(ex_id);
    exemplars.append_parent_id(parent_dp_id);
    exemplars.append_time_unix_nano(to_nanos(ex.time));
    match ex.value {
        NumberValue::Int(v) => {
            exemplars.append_int_value(Some(v));
            exemplars.append_double_value(None);
        }
        NumberValue::Float(v) => {
            exemplars.append_double_value(Some(v));
            exemplars.append_int_value(None);
        }
    }
    exemplars
        .append_span_id(ex.span_id.as_ref().unwrap_or(&[0u8; 8]))
        .expect("span id is 8 bytes");
    exemplars
        .append_trace_id(ex.trace_id.as_ref().unwrap_or(&[0u8; 16]))
        .expect("trace id is 16 bytes");
    for (k, v) in ex.attrs {
        v.append_to(exemplars_attr, ex_id, k.as_ref());
    }
    ex_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Resource, Scope, Value, ValueOwned, ZString, ZStringOwned};
    use arrow::array::AsArray;
    use arrow::datatypes::{UInt16Type, UInt32Type};
    use chrono::{DateTime, Utc};
    use otel_arrow_dfe_pdata::otlp::metrics::MetricType;
    use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
    use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::AggregationTemporality;
    use otel_arrow_dfe_pdata::schema::consts;
    use smol_str::SmolStr;
    use uuid::Uuid;

    fn scope() -> Scope {
        Scope {
            name: SmolStr::new_static("test"),
            listener_id: None,
            listener_type: None,
            listener_name: None,
            monitor_type: None,
            node_id: None,
        }
    }

    fn resource(host: &str) -> Resource {
        Resource {
            host_id: Uuid::nil(),
            host_name: SmolStr::new(host),
            host_timezone: None,
            endpoint_id: None,
            endpoint_name: None,
            endpoint_type: None,
            item_id: Uuid::nil(),
            item_name: SmolStr::new_static("item"),
        }
    }

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn rows(pl: &crate::payload::Payload, t: ArrowPayloadType) -> usize {
        pl.data.get(t).map(|b| b.num_rows()).unwrap_or(0)
    }

    fn parent_ids_u16(pl: &crate::payload::Payload, t: ArrowPayloadType) -> Vec<u16> {
        let b = pl.data.get(t).unwrap();
        b.column_by_name(consts::PARENT_ID)
            .unwrap()
            .as_primitive::<UInt16Type>()
            .values()
            .to_vec()
    }

    fn parent_ids_u32(pl: &crate::payload::Payload, t: ArrowPayloadType) -> Vec<u32> {
        let b = pl.data.get(t).unwrap();
        b.column_by_name(consts::PARENT_ID)
            .unwrap()
            .as_primitive::<UInt32Type>()
            .values()
            .to_vec()
    }

    #[test]
    fn empty_builder_finishes_empty() {
        let mut b = MetricsBuilder::new();
        assert!(b.is_empty());
        let pl = b.finish();
        assert!(pl.is_empty());
    }

    /// Push one gauge row carrying `n` data points.
    fn push_gauge(b: &mut MetricsBuilder, n: usize) {
        let dps: Vec<_> = (0..n)
            .map(|i| NumberDataPoint {
                start_time: None,
                time: ts(i as i64 + 1),
                value: NumberValue::Int(i as i64),
                flags: 0,
                attrs: &[],
                exemplars: &[],
            })
            .collect();
        b.push_row(MetricRow {
            name: "g".into(),
            r#type: MetricType::Gauge,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPoints::Gauge(Gauge { data_points: &dps }),
            attrs: &[],
        });
    }

    /// `(payload type, arrow schema, row count)` for every batch a payload carries —
    /// the part of the output that replacing the builders in `finish` could plausibly
    /// disturb. Deliberately not the values, which carry `Utc::now()` timestamps.
    fn shape(pl: &crate::payload::Payload) -> Vec<(String, String, usize)> {
        pl.data
            .allowed_payload_types()
            .iter()
            .filter_map(|&pt| {
                pl.data
                    .get(pt)
                    .map(|b| (format!("{pt:?}"), format!("{:?}", b.schema()), b.num_rows()))
            })
            .collect()
    }

    /// Push one row of every metric kind, with data-point attributes and exemplars, so
    /// that all fifteen sub-builders end up carrying rows.
    fn push_all_kinds(b: &mut MetricsBuilder) {
        let attrs = [(
            ZString::from_static("env"),
            Value::String(ZString::from_static("prod")),
        )];
        let ex = [Exemplar {
            time: ts(1),
            value: NumberValue::Int(1),
            span_id: Some([1u8; 8]),
            trace_id: Some([2u8; 16]),
            attrs: &attrs,
        }];

        let n_dps = [NumberDataPoint {
            start_time: Some(ts(1)),
            time: ts(2),
            value: NumberValue::Int(7),
            flags: 1,
            attrs: &attrs,
            exemplars: &ex,
        }];
        for (kind, dp) in [
            (
                MetricType::Gauge,
                DataPoints::Gauge(Gauge {
                    data_points: &n_dps,
                }),
            ),
            (
                MetricType::Sum,
                DataPoints::Sum(Sum {
                    data_points: &n_dps,
                    aggregation_temporality: AggregationTemporality::Cumulative,
                    is_monotonic: true,
                }),
            ),
        ] {
            b.push_row(MetricRow {
                name: "n".into(),
                r#type: kind,
                description: Some("d".into()),
                unit: Some("By".into()),
                source_ip: None,
                source_port: None,
                data_points: dp,
                attrs: &attrs,
            });
        }

        let quantiles = [Quantile {
            quantile: 0.5,
            value: 1.0,
        }];
        let s_dps = [SummaryDataPoint {
            start_time: Some(ts(1)),
            time: ts(2),
            count: 2,
            sum: 3.0,
            quantiles: &quantiles,
            flags: 1,
            attrs: &attrs,
        }];
        b.push_row(MetricRow {
            name: "s".into(),
            r#type: MetricType::Summary,
            description: Some("d".into()),
            unit: Some("By".into()),
            source_ip: None,
            source_port: None,
            data_points: DataPoints::Summary(Summary {
                data_points: &s_dps,
            }),
            attrs: &attrs,
        });

        let h_dps = [HistogramDataPoint {
            start_time: Some(ts(1)),
            time: ts(2),
            count: 6,
            sum: Some(10.0),
            bucket_counts: &[1, 2, 3],
            explicit_bounds: &[1.0, 2.0],
            flags: 1,
            min: Some(0.0),
            max: Some(5.0),
            attrs: &attrs,
            exemplars: &ex,
        }];
        b.push_row(MetricRow {
            name: "h".into(),
            r#type: MetricType::Histogram,
            description: Some("d".into()),
            unit: Some("By".into()),
            source_ip: None,
            source_port: None,
            data_points: DataPoints::Histogram(Histogram {
                data_points: &h_dps,
                aggregation_temporality: AggregationTemporality::Delta,
            }),
            attrs: &attrs,
        });

        let e_dps = [ExponentialHistogramDataPoint {
            start_time: Some(ts(1)),
            time: ts(2),
            count: 4,
            sum: Some(8.0),
            scale: 1,
            zero_count: 1,
            positive: Some(Buckets {
                offset: 0,
                bucket_counts: &[1, 2],
            }),
            negative: None,
            zero_threshold: 0.5,
            min: Some(0.0),
            max: Some(4.0),
            flags: 1,
            attrs: &attrs,
            exemplars: &ex,
        }];
        b.push_row(MetricRow {
            name: "e".into(),
            r#type: MetricType::ExponentialHistogram,
            description: Some("d".into()),
            unit: Some("By".into()),
            source_ip: None,
            source_port: None,
            data_points: DataPoints::ExponentialHistogram(ExponentialHistogram {
                data_points: &e_dps,
                aggregation_temporality: AggregationTemporality::Cumulative,
            }),
            attrs: &attrs,
        });
    }

    /// Reusing a builder has to be indistinguishable from starting a fresh one, which
    /// is what makes [`MetricsBuilder::reset_builders`] safe: it pins that the
    /// replacement neither drops a column nor adds one back, across all fifteen
    /// sub-builders.
    #[test]
    fn reused_builder_emits_the_same_shape_as_a_fresh_one() {
        fn batch(b: &mut MetricsBuilder) -> crate::payload::Payload {
            // A scope with an optional field set, so `scope_attr` contributes a batch
            // too — the shared `scope()` fixture is all-`None` and emits none.
            b.scope(Scope {
                monitor_type: Some(SmolStr::new_static("snmp_walk")),
                ..scope()
            });
            b.resource(resource("h"));
            push_all_kinds(b);
            b.finish()
        }

        let mut reused = MetricsBuilder::new();
        drop(batch(&mut reused));
        let second = batch(&mut reused);

        let first = batch(&mut MetricsBuilder::new());
        let want = shape(&first);
        // Guard against a vacuous pass: all fifteen sub-builders contributed a batch,
        // plus the three attribute builders `BaseBuilder::finish` owns.
        assert_eq!(
            want.len(),
            18,
            "expected all metric batches to be present, got {:?}",
            want.iter().map(|(t, ..)| t).collect::<Vec<_>>()
        );
        assert_eq!(shape(&second), want);
    }

    /// Push one gauge row with a single data point carrying `attrs`.
    fn push_gauge_with_dp_attrs(b: &mut MetricsBuilder, attrs: &[(ZString, Value)]) {
        let dps = [NumberDataPoint {
            start_time: None,
            time: ts(1),
            value: NumberValue::Int(1),
            flags: 0,
            attrs,
            exemplars: &[],
        }];
        b.push_row(MetricRow {
            name: "g".into(),
            r#type: MetricType::Gauge,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPoints::Gauge(Gauge { data_points: &dps }),
            attrs: &[],
        });
    }

    /// A reused builder must not carry per-column state from earlier batches.
    ///
    /// `is_monotonic` is an adaptive boolean column: gauge rows append nulls to it, and
    /// `finish` produces no array for an all-null/all-false column *without* dropping
    /// the nulls it counted. The first monotonic sum after that back-fills every one of
    /// them, making the column longer than the batch it belongs to.
    #[test]
    fn metric_bool_column_does_not_accumulate_across_batches() {
        let mut b = MetricsBuilder::new();
        b.scope(scope());
        b.resource(resource("h"));
        for _ in 0..4 {
            push_gauge(&mut b, 1);
        }
        assert_eq!(rows(&b.finish(), ArrowPayloadType::UnivariateMetrics), 4);

        // First metric to set `is_monotonic`, in a one-row batch.
        b.scope(scope());
        b.resource(resource("h"));
        let dps = [NumberDataPoint {
            start_time: None,
            time: ts(1),
            value: NumberValue::Int(1),
            flags: 0,
            attrs: &[],
            exemplars: &[],
        }];
        b.push_row(MetricRow {
            name: "s".into(),
            r#type: MetricType::Sum,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPoints::Sum(Sum {
                data_points: &dps,
                aggregation_temporality: AggregationTemporality::Cumulative,
                is_monotonic: true,
            }),
            attrs: &[],
        });
        let pl = b.finish();
        assert_eq!(
            rows(&pl, ArrowPayloadType::UnivariateMetrics),
            1,
            "metric batch must only hold this batch's rows"
        );
    }

    /// The same accumulation one table down: the data-point attribute builder's value
    /// columns that a batch of int-only attributes never writes to.
    #[test]
    fn dp_attr_value_columns_do_not_accumulate_across_batches() {
        let mut b = MetricsBuilder::new();
        b.scope(scope());
        b.resource(resource("h"));
        for _ in 0..8 {
            push_gauge_with_dp_attrs(&mut b, &[(ZString::from_static("k"), Value::Int64(1))]);
        }
        assert_eq!(rows(&b.finish(), ArrowPayloadType::NumberDpAttrs), 8);

        // First use of the `double` value column, in a one-row batch. 1.5 and not 0.0
        // because a column whose only values equal its default still counts as unused.
        b.scope(scope());
        b.resource(resource("h"));
        push_gauge_with_dp_attrs(&mut b, &[(ZString::from_static("k"), Value::Double(1.5))]);
        let pl = b.finish();
        assert_eq!(
            rows(&pl, ArrowPayloadType::NumberDpAttrs),
            1,
            "data-point attribute batch must only hold this batch's rows"
        );
    }

    /// `num_rows` counts metric rows, `num_items` counts data points. Batching
    /// compares `batch_size` against the latter, so the builder has to report it
    /// while it is still accumulating — and it must agree with what the finished
    /// payload reports.
    #[test]
    fn builder_counts_rows_and_data_points_separately() {
        let mut b = MetricsBuilder::new();
        b.resource(resource("h"));
        b.scope(scope());

        push_gauge(&mut b, 3);
        assert_eq!(b.num_rows(), 1);
        assert_eq!(b.num_items(), 3);

        push_gauge(&mut b, 4);
        assert_eq!(b.num_rows(), 2);
        assert_eq!(b.num_items(), 7);

        let pl = b.finish();
        assert_eq!(pl.num_rows(), 2, "builder rows == payload rows");
        assert_eq!(pl.num_items(), 7, "builder items == payload items");

        // finish() reset both counters.
        assert_eq!(b.num_rows(), 0);
        assert_eq!(b.num_items(), 0);
    }

    /// A metric row with no data points still exists as a row, so the builder
    /// must not look empty to a flush gate that only asks about items.
    #[test]
    fn builder_row_without_data_points_counts_as_a_row() {
        let mut b = MetricsBuilder::new();
        b.resource(resource("h"));
        b.scope(scope());
        push_gauge(&mut b, 0);

        assert_eq!(b.num_rows(), 1);
        assert_eq!(b.num_items(), 0);
        assert!(!b.is_empty());

        let pl = b.finish();
        assert_eq!(pl.num_rows(), 1);
        assert_eq!(pl.num_items(), 0);
    }

    #[test]
    fn builds_all_metric_kinds() {
        let mut b = MetricsBuilder::new();
        b.resource(resource("h"));
        b.scope(scope());

        // Gauge: two data points (id 0), no exemplars/attrs.
        let gauge_dps = [
            NumberDataPoint {
                start_time: None,
                time: ts(1),
                value: NumberValue::Int(7),
                flags: 0,
                attrs: &[],
                exemplars: &[],
            },
            NumberDataPoint {
                start_time: Some(ts(0)),
                time: ts(2),
                value: NumberValue::Float(1.5),
                flags: 0,
                attrs: &[],
                exemplars: &[],
            },
        ];
        b.push_row(MetricRow {
            name: "g".into(),
            r#type: MetricType::Gauge,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPoints::Gauge(Gauge {
                data_points: &gauge_dps,
            }),
            attrs: &[],
        });

        // Sum (id 1): one data point with a data-point attr and one exemplar,
        // plus a metric-level attr.
        let sum_ex = [Exemplar {
            time: ts(1),
            value: NumberValue::Int(7),
            span_id: Some([1u8; 8]),
            trace_id: Some([2u8; 16]),
            attrs: &[],
        }];
        let sum_dp_attrs = [(ZString::from_static("k"), Value::Int64(1))];
        let sum_dps = [NumberDataPoint {
            start_time: None,
            time: ts(3),
            value: NumberValue::Int(7),
            flags: 0,
            attrs: &sum_dp_attrs,
            exemplars: &sum_ex,
        }];
        let metric_attrs = [(
            ZString::from_static("env"),
            Value::String(ZString::from_static("prod")),
        )];
        b.push_row(MetricRow {
            name: "s".into(),
            r#type: MetricType::Sum,
            description: Some("desc".into()),
            unit: Some("By".into()),
            source_ip: None,
            source_port: None,
            data_points: DataPoints::Sum(Sum {
                data_points: &sum_dps,
                aggregation_temporality: AggregationTemporality::Cumulative,
                is_monotonic: true,
            }),
            attrs: &metric_attrs,
        });

        // Histogram (id 2): one data point, no exemplars.
        let h_dps = [HistogramDataPoint {
            start_time: None,
            time: ts(4),
            count: 6,
            sum: Some(10.0),
            bucket_counts: &[1, 2, 3],
            explicit_bounds: &[1.0, 2.0],
            flags: 0,
            min: Some(0.0),
            max: Some(5.0),
            attrs: &[],
            exemplars: &[],
        }];
        b.push_row(MetricRow {
            name: "h".into(),
            r#type: MetricType::Histogram,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPoints::Histogram(Histogram {
                data_points: &h_dps,
                aggregation_temporality: AggregationTemporality::Delta,
            }),
            attrs: &[],
        });

        // ExponentialHistogram (id 3): positive + negative buckets and one exemplar.
        let e_ex = [Exemplar {
            time: ts(4),
            value: NumberValue::Float(2.0),
            span_id: None,
            trace_id: None,
            attrs: &[],
        }];
        let e_dps = [ExponentialHistogramDataPoint {
            start_time: None,
            time: ts(5),
            count: 6,
            sum: Some(3.0),
            scale: 2,
            zero_count: 1,
            positive: Some(Buckets {
                offset: 0,
                bucket_counts: &[1, 2],
            }),
            negative: Some(Buckets {
                offset: -1,
                bucket_counts: &[3],
            }),
            zero_threshold: 0.0,
            min: None,
            max: None,
            flags: 0,
            attrs: &[],
            exemplars: &e_ex,
        }];
        b.push_row(MetricRow {
            name: "e".into(),
            r#type: MetricType::ExponentialHistogram,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPoints::ExponentialHistogram(ExponentialHistogram {
                data_points: &e_dps,
                aggregation_temporality: AggregationTemporality::Cumulative,
            }),
            attrs: &[],
        });

        // Summary (id 4): one data point with quantiles.
        let q = [
            Quantile {
                quantile: 0.5,
                value: 1.0,
            },
            Quantile {
                quantile: 0.99,
                value: 9.0,
            },
        ];
        let su_dps = [SummaryDataPoint {
            start_time: None,
            time: ts(6),
            count: 4,
            sum: 12.0,
            quantiles: &q,
            flags: 0,
            attrs: &[],
        }];
        b.push_row(MetricRow {
            name: "su".into(),
            r#type: MetricType::Summary,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPoints::Summary(Summary {
                data_points: &su_dps,
            }),
            attrs: &[],
        });

        // Owned Gauge (id 5): exercises the owned path — metric attr, dp attr,
        // and an exemplar with its own attr.
        b.push_row_owned(MetricRowOwned {
            name: ZStringOwned::from_static("og"),
            r#type: MetricType::Gauge,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPointsOwned::Gauge(GaugeOwned {
                data_points: vec![NumberDataPointOwned {
                    start_time: None,
                    time: ts(7),
                    value: NumberValue::Float(3.0),
                    flags: 0,
                    attrs: vec![(ZStringOwned::from_static("dk"), ValueOwned::Int64(2))],
                    exemplars: vec![ExemplarOwned {
                        time: ts(7),
                        value: NumberValue::Float(3.0),
                        span_id: None,
                        trace_id: None,
                        attrs: vec![(ZStringOwned::from_static("ek"), ValueOwned::Bool(true))],
                    }],
                }],
            }),
            attrs: vec![(
                ZStringOwned::from_static("env"),
                ValueOwned::String(ZStringOwned::from_static("dev")),
            )],
        });

        let pl = b.finish();

        // Root + per-type data point counts.
        assert_eq!(rows(&pl, ArrowPayloadType::UnivariateMetrics), 6);
        assert_eq!(rows(&pl, ArrowPayloadType::NumberDataPoints), 4); // gauge 2 + sum 1 + owned 1
        assert_eq!(rows(&pl, ArrowPayloadType::SummaryDataPoints), 1);
        assert_eq!(rows(&pl, ArrowPayloadType::HistogramDataPoints), 1);
        assert_eq!(rows(&pl, ArrowPayloadType::ExpHistogramDataPoints), 1);

        // Exemplars.
        assert_eq!(rows(&pl, ArrowPayloadType::NumberDpExemplars), 2); // sum + owned
        assert_eq!(rows(&pl, ArrowPayloadType::ExpHistogramDpExemplars), 1);
        assert_eq!(rows(&pl, ArrowPayloadType::HistogramDpExemplars), 0);

        // Attribute tables.
        assert_eq!(rows(&pl, ArrowPayloadType::MetricAttrs), 2); // sum + owned metric attrs
        assert_eq!(rows(&pl, ArrowPayloadType::NumberDpAttrs), 2); // sum dp + owned dp attrs
        assert_eq!(rows(&pl, ArrowPayloadType::NumberDpExemplarAttrs), 1); // owned exemplar attr
        assert!(rows(&pl, ArrowPayloadType::ResourceAttrs) > 0);

        // Wiring: number-dp parent ids point at metric ids; the gauge (id 0)
        // owns two of the four data points.
        let ndp_parents = parent_ids_u16(&pl, ArrowPayloadType::NumberDataPoints);
        assert_eq!(ndp_parents, vec![0, 0, 1, 5]);

        // Exemplar parent ids point at number-dp ids (u32): sum's dp (2) and the
        // owned gauge's dp (3).
        let ndpe_parents = parent_ids_u32(&pl, ArrowPayloadType::NumberDpExemplars);
        assert_eq!(ndpe_parents, vec![2, 3]);
    }

    #[test]
    fn finish_resets_for_reuse() {
        let mut b = MetricsBuilder::new();
        b.scope(scope());
        let dps = [NumberDataPoint {
            start_time: None,
            time: ts(1),
            value: NumberValue::Int(1),
            flags: 0,
            attrs: &[],
            exemplars: &[],
        }];
        let row = || MetricRow {
            name: "g".into(),
            r#type: MetricType::Gauge,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPoints::Gauge(Gauge { data_points: &dps }),
            attrs: &[],
        };
        b.push_row(row());
        b.push_row(row());
        let pl = b.finish();
        assert_eq!(rows(&pl, ArrowPayloadType::UnivariateMetrics), 2);
        assert_eq!(rows(&pl, ArrowPayloadType::NumberDataPoints), 2);

        // The builder resets its bookkeeping (row id + data-point counters) so the
        // next batch starts clean.
        assert!(b.is_empty());
        b.push_row(row());
        let pl2 = b.finish();
        assert_eq!(rows(&pl2, ArrowPayloadType::UnivariateMetrics), 1);
        assert_eq!(rows(&pl2, ArrowPayloadType::NumberDataPoints), 1);
        // Metric id restarted at 0 → the lone data point parents to 0.
        assert_eq!(
            parent_ids_u16(&pl2, ArrowPayloadType::NumberDataPoints),
            vec![0]
        );
    }

    /// Exercises the owned (`push_row_owned`) data-point helpers for every kind
    /// that carries exemplars/quantiles, which the borrowed test doesn't cover.
    #[test]
    fn owned_path_all_kinds() {
        let mut b = MetricsBuilder::new();
        b.resource(resource("h"));
        b.scope(scope());

        // Owned Sum (id 0): one dp with an exemplar.
        b.push_row_owned(MetricRowOwned {
            name: ZStringOwned::from_static("s"),
            r#type: MetricType::Sum,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPointsOwned::Sum(SumOwned {
                data_points: vec![NumberDataPointOwned {
                    start_time: None,
                    time: ts(1),
                    value: NumberValue::Int(5),
                    flags: 0,
                    attrs: vec![],
                    exemplars: vec![ExemplarOwned {
                        time: ts(1),
                        value: NumberValue::Int(5),
                        span_id: None,
                        trace_id: None,
                        attrs: vec![],
                    }],
                }],
                aggregation_temporality: AggregationTemporality::Cumulative,
                is_monotonic: true,
            }),
            attrs: vec![],
        });

        // Owned Histogram (id 1): one dp with buckets and an exemplar.
        b.push_row_owned(MetricRowOwned {
            name: ZStringOwned::from_static("h"),
            r#type: MetricType::Histogram,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPointsOwned::Histogram(HistogramOwned {
                data_points: vec![HistogramDataPointOwned {
                    start_time: None,
                    time: ts(2),
                    count: 3,
                    sum: Some(6.0),
                    bucket_counts: vec![1, 2],
                    explicit_bounds: vec![1.0],
                    flags: 0,
                    min: Some(0.0),
                    max: Some(2.0),
                    attrs: vec![],
                    exemplars: vec![ExemplarOwned {
                        time: ts(2),
                        value: NumberValue::Float(1.0),
                        span_id: None,
                        trace_id: None,
                        attrs: vec![],
                    }],
                }],
                aggregation_temporality: AggregationTemporality::Delta,
            }),
            attrs: vec![],
        });

        // Owned ExponentialHistogram (id 2): pos/neg buckets and an exemplar.
        b.push_row_owned(MetricRowOwned {
            name: ZStringOwned::from_static("e"),
            r#type: MetricType::ExponentialHistogram,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPointsOwned::ExponentialHistogram(ExponentialHistogramOwned {
                data_points: vec![ExponentialHistogramDataPointOwned {
                    start_time: None,
                    time: ts(3),
                    count: 4,
                    sum: Some(2.0),
                    scale: 1,
                    zero_count: 0,
                    positive: Some(BucketsOwned {
                        offset: 0,
                        bucket_counts: vec![1, 1],
                    }),
                    negative: Some(BucketsOwned {
                        offset: -2,
                        bucket_counts: vec![2],
                    }),
                    zero_threshold: 0.0,
                    min: None,
                    max: None,
                    flags: 0,
                    attrs: vec![],
                    exemplars: vec![ExemplarOwned {
                        time: ts(3),
                        value: NumberValue::Float(1.0),
                        span_id: None,
                        trace_id: None,
                        attrs: vec![],
                    }],
                }],
                aggregation_temporality: AggregationTemporality::Cumulative,
            }),
            attrs: vec![],
        });

        // Owned Summary (id 3): one dp with quantiles.
        b.push_row_owned(MetricRowOwned {
            name: ZStringOwned::from_static("su"),
            r#type: MetricType::Summary,
            description: None,
            unit: None,
            source_ip: None,
            source_port: None,
            data_points: DataPointsOwned::Summary(SummaryOwned {
                data_points: vec![SummaryDataPointOwned {
                    start_time: None,
                    time: ts(4),
                    count: 2,
                    sum: 8.0,
                    quantiles: vec![Quantile {
                        quantile: 0.5,
                        value: 4.0,
                    }],
                    flags: 0,
                    attrs: vec![],
                }],
            }),
            attrs: vec![],
        });

        let pl = b.finish();

        assert_eq!(rows(&pl, ArrowPayloadType::UnivariateMetrics), 4);
        assert_eq!(rows(&pl, ArrowPayloadType::NumberDataPoints), 1);
        assert_eq!(rows(&pl, ArrowPayloadType::HistogramDataPoints), 1);
        assert_eq!(rows(&pl, ArrowPayloadType::ExpHistogramDataPoints), 1);
        assert_eq!(rows(&pl, ArrowPayloadType::SummaryDataPoints), 1);

        assert_eq!(rows(&pl, ArrowPayloadType::NumberDpExemplars), 1);
        assert_eq!(rows(&pl, ArrowPayloadType::HistogramDpExemplars), 1);
        assert_eq!(rows(&pl, ArrowPayloadType::ExpHistogramDpExemplars), 1);

        // Each data point parents to its own metric id.
        assert_eq!(
            parent_ids_u16(&pl, ArrowPayloadType::NumberDataPoints),
            vec![0]
        );
        assert_eq!(
            parent_ids_u16(&pl, ArrowPayloadType::HistogramDataPoints),
            vec![1]
        );
        assert_eq!(
            parent_ids_u16(&pl, ArrowPayloadType::ExpHistogramDataPoints),
            vec![2]
        );
        assert_eq!(
            parent_ids_u16(&pl, ArrowPayloadType::SummaryDataPoints),
            vec![3]
        );
        // Each kind's exemplar parents to its (per-table) data-point id 0.
        assert_eq!(
            parent_ids_u32(&pl, ArrowPayloadType::HistogramDpExemplars),
            vec![0]
        );
        assert_eq!(
            parent_ids_u32(&pl, ArrowPayloadType::ExpHistogramDpExemplars),
            vec![0]
        );
    }

    /// The owned path appends attributes straight from the owned slices rather than
    /// through borrowed views, so pin it to the borrowed path: the same rows pushed
    /// either way must produce identical tables, attribute parent ids included.
    #[test]
    fn owned_path_matches_borrowed_path() {
        type Attrs = Vec<(ZStringOwned, ValueOwned)>;
        fn kv(pairs: &[(&'static str, i64)]) -> Attrs {
            pairs
                .iter()
                .map(|&(k, v)| (ZStringOwned::from_static(k), ValueOwned::Int64(v)))
                .collect()
        }
        fn exemplars(n: i64) -> Vec<ExemplarOwned> {
            (0..2)
                .map(|i| ExemplarOwned {
                    time: ts(n + i),
                    value: NumberValue::Int(n * 10 + i),
                    span_id: Some([i as u8; 8]),
                    trace_id: None,
                    attrs: kv(&[("ek", n), ("ek2", i)]),
                })
                .collect()
        }
        fn row(
            name: &'static str,
            r#type: MetricType,
            data_points: DataPointsOwned,
        ) -> MetricRowOwned {
            MetricRowOwned {
                name: ZStringOwned::from_static(name),
                r#type,
                description: Some(ZStringOwned::from_static("d")),
                unit: None,
                source_ip: None,
                source_port: Some(9100),
                data_points,
                attrs: kv(&[("rk", 1)]),
            }
        }
        let number = |n: i64| NumberDataPointOwned {
            start_time: None,
            time: ts(n),
            value: NumberValue::Int(n),
            flags: 0,
            attrs: kv(&[("dk", n), ("dk2", -n)]),
            exemplars: exemplars(n),
        };
        let rows = vec![
            row(
                "s",
                MetricType::Sum,
                DataPointsOwned::Sum(SumOwned {
                    data_points: vec![number(1), number(2)],
                    aggregation_temporality: AggregationTemporality::Cumulative,
                    is_monotonic: true,
                }),
            ),
            row(
                "g",
                MetricType::Gauge,
                DataPointsOwned::Gauge(GaugeOwned {
                    data_points: vec![number(3)],
                }),
            ),
            row(
                "h",
                MetricType::Histogram,
                DataPointsOwned::Histogram(HistogramOwned {
                    data_points: (4..6)
                        .map(|n| HistogramDataPointOwned {
                            start_time: Some(ts(0)),
                            time: ts(n),
                            count: 3,
                            sum: Some(6.0),
                            bucket_counts: vec![1, 2],
                            explicit_bounds: vec![1.0],
                            flags: 0,
                            min: None,
                            max: Some(2.0),
                            attrs: kv(&[("hk", n)]),
                            exemplars: exemplars(n),
                        })
                        .collect(),
                    aggregation_temporality: AggregationTemporality::Delta,
                }),
            ),
            row(
                "e",
                MetricType::ExponentialHistogram,
                DataPointsOwned::ExponentialHistogram(ExponentialHistogramOwned {
                    data_points: (6..8)
                        .map(|n| ExponentialHistogramDataPointOwned {
                            start_time: None,
                            time: ts(n),
                            count: 4,
                            sum: Some(2.0),
                            scale: 1,
                            zero_count: 0,
                            positive: Some(BucketsOwned {
                                offset: 0,
                                bucket_counts: vec![1, 1],
                            }),
                            negative: None,
                            zero_threshold: 0.0,
                            min: None,
                            max: None,
                            flags: 0,
                            attrs: kv(&[("xk", n)]),
                            exemplars: exemplars(n),
                        })
                        .collect(),
                    aggregation_temporality: AggregationTemporality::Cumulative,
                }),
            ),
            row(
                "su",
                MetricType::Summary,
                DataPointsOwned::Summary(SummaryOwned {
                    data_points: (8..10)
                        .map(|n| SummaryDataPointOwned {
                            start_time: None,
                            time: ts(n),
                            count: 2,
                            sum: 8.0,
                            quantiles: vec![Quantile {
                                quantile: 0.5,
                                value: 4.0,
                            }],
                            flags: 0,
                            attrs: kv(&[("qk", n), ("qk2", n)]),
                        })
                        .collect(),
                }),
            ),
        ];

        let mut owned = MetricsBuilder::new();
        owned.resource(resource("h"));
        owned.scope(scope());
        for r in rows.clone() {
            owned.push_row_owned(r);
        }
        let owned = owned.finish();

        let mut borrowed = MetricsBuilder::new();
        borrowed.resource(resource("h"));
        borrowed.scope(scope());
        for r in &rows {
            push_borrowed(&mut borrowed, r);
        }
        let borrowed = borrowed.finish();

        assert_eq!(shape(&owned), shape(&borrowed));
        for &pt in owned.data.allowed_payload_types() {
            assert_eq!(owned.data.get(pt), borrowed.data.get(pt), "{pt:?}");
        }
        assert_eq!(rows_of_attrs(&owned), [6, 10, 4, 2, 8, 2, 8, 12]);
    }

    /// Attribute row counts, in a fixed order, so the equality above can't pass
    /// vacuously on two empty tables.
    fn rows_of_attrs(pl: &crate::payload::Payload) -> Vec<usize> {
        [
            ArrowPayloadType::NumberDpAttrs,
            ArrowPayloadType::MetricAttrs,
            ArrowPayloadType::SummaryDpAttrs,
            ArrowPayloadType::HistogramDpAttrs,
            ArrowPayloadType::HistogramDpExemplarAttrs,
            ArrowPayloadType::ExpHistogramDpAttrs,
            ArrowPayloadType::ExpHistogramDpExemplarAttrs,
            ArrowPayloadType::NumberDpExemplarAttrs,
        ]
        .map(|pt| rows(pl, pt))
        .to_vec()
    }

    /// Pushes an owned row through the borrowed `push_row`, by collecting borrowed
    /// views of everything it owns.
    fn push_borrowed(b: &mut MetricsBuilder, row: &MetricRowOwned) {
        fn view(a: &[(ZStringOwned, ValueOwned)]) -> Vec<(ZString<'_>, Value<'_>)> {
            a.iter()
                .map(|(k, v)| (k.as_ref_z(), v.as_ref_z()))
                .collect()
        }
        fn buckets(b: &Option<BucketsOwned>) -> Option<Buckets<'_>> {
            b.as_ref().map(|b| Buckets {
                offset: b.offset,
                bucket_counts: &b.bucket_counts,
            })
        }
        fn ex_views<'a>(
            ex: &'a [ExemplarOwned],
            attrs: &'a [Vec<(ZString<'a>, Value<'a>)>],
        ) -> Vec<Exemplar<'a>> {
            ex.iter()
                .zip(attrs)
                .map(|(e, attrs)| Exemplar {
                    time: e.time,
                    value: e.value,
                    span_id: e.span_id,
                    trace_id: e.trace_id,
                    attrs,
                })
                .collect()
        }
        macro_rules! dp_views {
            ($dps:expr) => {{
                let attrs: Vec<_> = $dps.iter().map(|d| view(&d.attrs)).collect();
                let ex_attrs: Vec<Vec<_>> = $dps
                    .iter()
                    .map(|d| d.exemplars.iter().map(|e| view(&e.attrs)).collect())
                    .collect();
                (attrs, ex_attrs)
            }};
        }
        let row_attrs = view(&row.attrs);
        let push = |b: &mut MetricsBuilder, data_points: DataPoints| {
            b.push_row(MetricRow {
                name: row.name.as_ref().into(),
                r#type: row.r#type,
                description: row.description.as_ref().map(|d| d.as_ref().into()),
                unit: row.unit.as_ref().map(|u| u.as_ref().into()),
                source_ip: row.source_ip,
                source_port: row.source_port,
                data_points,
                attrs: &row_attrs,
            })
        };
        match &row.data_points {
            DataPointsOwned::Sum(SumOwned {
                data_points: dps, ..
            })
            | DataPointsOwned::Gauge(GaugeOwned { data_points: dps }) => {
                let (attrs, ex_attrs) = dp_views!(dps);
                let exs: Vec<_> = dps
                    .iter()
                    .zip(&ex_attrs)
                    .map(|(d, a)| ex_views(&d.exemplars, a))
                    .collect();
                let views: Vec<_> = dps
                    .iter()
                    .enumerate()
                    .map(|(i, d)| NumberDataPoint {
                        start_time: d.start_time,
                        time: d.time,
                        value: d.value,
                        flags: d.flags,
                        attrs: &attrs[i],
                        exemplars: &exs[i],
                    })
                    .collect();
                let data_points = match &row.data_points {
                    DataPointsOwned::Sum(s) => DataPoints::Sum(Sum {
                        data_points: &views,
                        aggregation_temporality: s.aggregation_temporality,
                        is_monotonic: s.is_monotonic,
                    }),
                    _ => DataPoints::Gauge(Gauge {
                        data_points: &views,
                    }),
                };
                push(b, data_points);
            }
            DataPointsOwned::Histogram(h) => {
                let dps = &h.data_points;
                let (attrs, ex_attrs) = dp_views!(dps);
                let exs: Vec<_> = dps
                    .iter()
                    .zip(&ex_attrs)
                    .map(|(d, a)| ex_views(&d.exemplars, a))
                    .collect();
                let views: Vec<_> = dps
                    .iter()
                    .enumerate()
                    .map(|(i, d)| HistogramDataPoint {
                        start_time: d.start_time,
                        time: d.time,
                        count: d.count,
                        sum: d.sum,
                        bucket_counts: &d.bucket_counts,
                        explicit_bounds: &d.explicit_bounds,
                        flags: d.flags,
                        min: d.min,
                        max: d.max,
                        attrs: &attrs[i],
                        exemplars: &exs[i],
                    })
                    .collect();
                push(
                    b,
                    DataPoints::Histogram(Histogram {
                        data_points: &views,
                        aggregation_temporality: h.aggregation_temporality,
                    }),
                );
            }
            DataPointsOwned::ExponentialHistogram(e) => {
                let dps = &e.data_points;
                let (attrs, ex_attrs) = dp_views!(dps);
                let exs: Vec<_> = dps
                    .iter()
                    .zip(&ex_attrs)
                    .map(|(d, a)| ex_views(&d.exemplars, a))
                    .collect();
                let views: Vec<_> = dps
                    .iter()
                    .enumerate()
                    .map(|(i, d)| ExponentialHistogramDataPoint {
                        start_time: d.start_time,
                        time: d.time,
                        count: d.count,
                        sum: d.sum,
                        scale: d.scale,
                        zero_count: d.zero_count,
                        positive: buckets(&d.positive),
                        negative: buckets(&d.negative),
                        zero_threshold: d.zero_threshold,
                        min: d.min,
                        max: d.max,
                        flags: d.flags,
                        attrs: &attrs[i],
                        exemplars: &exs[i],
                    })
                    .collect();
                push(
                    b,
                    DataPoints::ExponentialHistogram(ExponentialHistogram {
                        data_points: &views,
                        aggregation_temporality: e.aggregation_temporality,
                    }),
                );
            }
            DataPointsOwned::Summary(s) => {
                let dps = &s.data_points;
                let attrs: Vec<_> = dps.iter().map(|d| view(&d.attrs)).collect();
                let views: Vec<_> = dps
                    .iter()
                    .enumerate()
                    .map(|(i, d)| SummaryDataPoint {
                        start_time: d.start_time,
                        time: d.time,
                        count: d.count,
                        sum: d.sum,
                        quantiles: &d.quantiles,
                        flags: d.flags,
                        attrs: &attrs[i],
                    })
                    .collect();
                push(
                    b,
                    DataPoints::Summary(Summary {
                        data_points: &views,
                    }),
                );
            }
        }
    }
}
