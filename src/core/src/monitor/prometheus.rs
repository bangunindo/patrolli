// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::cache::item::Stats;
use crate::common;
use crate::config::Tls;
use crate::config::endpoint::{Config as EndpointConfig, Endpoint, HttpAuth};
use crate::config::host::Host;
use crate::config::item::{Config as ItemConfig, Item, ItemKind};
use crate::monitor::MonitorRecord;
use crate::payload::{
    BucketsOwned, DataPointsOwned, ExemplarOwned, ExponentialHistogramDataPointOwned,
    ExponentialHistogramOwned, GaugeOwned, HistogramDataPointOwned, HistogramOwned, MetricRowOwned,
    NumberDataPointOwned, NumberValue, Quantile, SumOwned, SummaryDataPointOwned, SummaryOwned,
    ValueOwned, ZStringOwned,
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use otel_arrow_dfe_pdata::otlp::metrics::MetricType;
use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::AggregationTemporality;
use prometheus_scraper::client::{Certificate, Identity};
use prometheus_scraper::{Client, ClientBuilder, ScrapeError, owned};
use std::mem::discriminant;
use std::sync::Arc;
use tracing::debug;

pub async fn process_prometheus_scrape(
    item: &Item,
    endpoint: &Endpoint,
    host: &Host,
    stats: &Arc<Stats>,
    sender: &flume::Sender<MonitorRecord>,
) -> Result<super::Collected, super::Error> {
    let EndpointConfig::Http(ep_conf) = &endpoint.config else {
        return Err(super::Error::config(format!(
            "not a http endpoint: {:?}",
            discriminant(&endpoint.config),
        )));
    };
    let ItemConfig::PrometheusScrape(prom_conf) = &item.config else {
        return Err(super::Error::config(format!(
            "not a prometheus scrape config: {:?}",
            discriminant(&item.config),
        )));
    };

    // Single timestamp reused for every data point that lacks an explicit sample
    // timestamp — matches the OTel "use the scrape time" rule.
    let scrape_time = Utc::now();

    let mut builder = Client::builder(ep_conf.url.to_string())
        .accept_protobuf(prom_conf.accept_protobuf)
        .connect_timeout(ep_conf.connect_timeout);
    match &ep_conf.auth {
        Some(HttpAuth::Basic { username, password }) => {
            let password = password
                .expose()
                .map_err(|e| super::Error::config(format!("basic auth password: {e}")))?;
            builder = builder.basic_auth(username, password);
        }
        Some(HttpAuth::Bearer(token)) => {
            let token = token
                .expose()
                .map_err(|e| super::Error::config(format!("bearer token: {e}")))?;
            builder = builder.bearer_token(token);
        }
        None => {}
    }
    if let Some(tls) = &ep_conf.tls {
        builder = apply_tls(builder, tls)?;
    }

    let client = builder
        .build()
        .map_err(|e| super::Error::config(format!("failed to build scrape client: {e}")))?;
    let mut stream = client.scrape().await.map_err(map_scrape_err)?;

    let resource = super::create_resource(host, Some(endpoint), item);
    let kind = ItemKind::from(&item.config);
    let processors = item.processors.clone();

    let mut batch: Vec<super::MonitorMetric> = Vec::new();
    // Counted here rather than derived from `batch`, because in streaming mode the
    // rows leave through `sender` and never appear in the returned record.
    let mut rows = 0u64;
    while let Some(scraped) = stream.next().await {
        let family = match scraped {
            Ok(family) => family,
            // Per the crate's framing contract a parse error is isolated to one
            // family; skip it and keep going.
            Err(ScrapeError::Parse(e)) => {
                debug!(error = %e, "skipping unparseable metric family");
                continue;
            }
            // A transport error ends the stream. Classify it the same way as a
            // failure to open the stream — a 5xx mid-scrape is no less a target
            // error for having arrived late.
            Err(e) => return Err(map_scrape_err(e)),
        };
        let Some(row) = family_to_row(family, scrape_time) else {
            continue;
        };
        let record = super::MonitorMetric {
            row,
            resource: resource.clone(),
            kind,
            processors: processors.clone(),
            stats: stats.clone(),
        };
        rows += 1;
        if prom_conf.stream {
            sender
                .send_async(MonitorRecord::SingleMetric(Box::new(record)))
                .await
                .map_err(|e| super::Error::dropped(e.to_string()))?;
        } else {
            batch.push(record);
        }
    }

    if batch.is_empty() {
        // Streaming already sent everything (or there was nothing to send).
        Ok(super::Collected::new(MonitorRecord::Null, rows))
    } else {
        Ok(super::Collected::new(
            MonitorRecord::BatchMetric(batch),
            rows,
        ))
    }
}

/// Maps the endpoint [`Tls`] config onto the scrape client builder. Only the knobs
/// `prometheus_scraper` exposes (reqwest's rustls backend) are available: a client
/// identity for mTLS, extra root certificates, and the insecure escape hatch.
fn apply_tls(mut builder: ClientBuilder, tls: &Tls) -> Result<ClientBuilder, super::Error> {
    if tls.insecure_skip_verify {
        builder = builder.danger_accept_invalid_certs(true);
    }
    if let Some(ca_path) = &tls.ca_cert_path {
        let pem = std::fs::read(ca_path)
            .map_err(|e| super::Error::config(format!("read CA cert {ca_path}: {e}")))?;
        let certs = Certificate::from_pem_bundle(&pem)
            .map_err(|e| super::Error::config(format!("parse CA cert {ca_path}: {e}")))?;
        for cert in certs {
            builder = builder.add_root_certificate(cert);
        }
    }
    match (&tls.cert_path, &tls.key_path) {
        (Some(cert_path), Some(key_path)) => {
            let cert_pem = std::fs::read(cert_path)
                .map_err(|e| super::Error::config(format!("read client cert {cert_path}: {e}")))?;
            // reqwest's rustls `Identity::from_pem` wants the cert chain and the
            // (plaintext) key in one PEM blob; `load_private_key_pem` decrypts an
            // encrypted PKCS#8 key for us.
            let key_password = tls
                .key_password()
                .map_err(|e| super::Error::config(format!("load client key: {e}")))?;
            let key_pem = common::tls::load_private_key_pem(key_path, key_password)
                .map_err(|e| super::Error::config(format!("load client key: {e}")))?;
            let mut combined = cert_pem;
            combined.push(b'\n');
            combined.extend_from_slice(&key_pem);
            let identity = Identity::from_pem(&combined)
                .map_err(|e| super::Error::config(format!("build client identity: {e}")))?;
            builder = builder.identity(identity);
        }
        (None, None) => {}
        _ => {
            return Err(super::Error::config(
                "mTLS misconfiguration: both cert_path and key_path must be set",
            ));
        }
    }
    Ok(builder)
}

/// Classify a scrape failure into the kind the stats page shows, which is also
/// what decides whether the check is retried.
///
/// Applied both to opening the stream and to a transport error part-way through
/// it: a 5xx mid-scrape is no less a target error for having arrived late.
fn map_scrape_err(e: ScrapeError) -> super::Error {
    let msg = e.to_string();
    match &e {
        // Bad TLS material or an unusable client config — a human has to fix it.
        ScrapeError::Build(_) => super::Error::config(msg),
        ScrapeError::Http(inner) => {
            if inner.is_timeout() {
                super::Error::timeout(msg)
            } else if inner.is_decode() {
                super::Error::malformed(msg)
            } else {
                // Connect, TLS handshake and body I/O all mean we couldn't get a
                // usable answer off the wire.
                super::Error::unreachable(msg)
            }
        }
        ScrapeError::Status(s) => match s.as_u16() {
            401 | 403 => super::Error::auth(msg),
            // The exporter is up but this path isn't it: a wrong URL, not a sick
            // target.
            404 => super::Error::config(msg),
            _ => super::Error::target(msg),
        },
        ScrapeError::Parse(_) => super::Error::malformed(msg),
        // We don't use `scrape_for_each`, so this can only be our own bug.
        ScrapeError::User(_) => super::Error::internal(msg),
        // `ScrapeError` is non-exhaustive; a variant added upstream is something
        // we don't model yet, so keep it retryable rather than silently calling
        // it a config problem the user has to go and fix.
        _ => super::Error::target(msg),
    }
}

// ---------------------------------------------------------------------------
// Prometheus family -> OTLP metric row conversion
//
// Mirrors the OpenTelemetry Prometheus/OpenMetrics compatibility spec and the
// collector-contrib prometheusreceiver. The 9 Prometheus/OpenMetrics types
// collapse into the 5 OTLP shapes (Sum / Gauge / Histogram / ExponentialHistogram
// / Summary). The scraper crate has already folded `_bucket`/`_count`/`_sum` and
// stripped the `le`/`quantile` labels into structured buckets/quantiles.
// ---------------------------------------------------------------------------

/// Convert one parsed family into a single [`MetricRowOwned`] (one OTLP metric with
/// one data point per series). Returns `None` for families that are dropped (gauge
/// histograms, unsupported native schemas, or families that ended up empty).
fn family_to_row(
    family: owned::MetricFamily,
    scrape_time: DateTime<Utc>,
) -> Option<MetricRowOwned> {
    use owned::MetricType as T;

    let owned::MetricFamily {
        name,
        help,
        r#type,
        metric,
        unit,
    } = family;

    let (otlp_type, data_points) = match r#type {
        T::Counter => {
            let dps: Vec<_> = metric
                .into_iter()
                .filter_map(|m| counter_dp(m, scrape_time))
                .collect();
            if dps.is_empty() {
                return None;
            }
            (
                MetricType::Sum,
                DataPointsOwned::Sum(SumOwned {
                    data_points: dps,
                    aggregation_temporality: AggregationTemporality::Cumulative,
                    is_monotonic: true,
                }),
            )
        }
        T::Gauge | T::Untyped => {
            let dps: Vec<_> = metric
                .into_iter()
                .filter_map(|m| gauge_dp(m, scrape_time))
                .collect();
            if dps.is_empty() {
                return None;
            }
            (
                MetricType::Gauge,
                DataPointsOwned::Gauge(GaugeOwned { data_points: dps }),
            )
        }
        T::Summary => {
            let dps: Vec<_> = metric
                .into_iter()
                .filter_map(|m| summary_dp(m, scrape_time))
                .collect();
            if dps.is_empty() {
                return None;
            }
            (
                MetricType::Summary,
                DataPointsOwned::Summary(SummaryOwned { data_points: dps }),
            )
        }
        T::Histogram => {
            let dps: Vec<_> = metric
                .into_iter()
                .filter_map(|m| histogram_dp(m, scrape_time))
                .collect();
            if dps.is_empty() {
                return None;
            }
            (
                MetricType::Histogram,
                DataPointsOwned::Histogram(HistogramOwned {
                    data_points: dps,
                    aggregation_temporality: AggregationTemporality::Cumulative,
                }),
            )
        }
        T::NativeHistogram | T::HybridHistogram => {
            let dps: Vec<_> = metric
                .into_iter()
                .filter_map(|m| exp_histogram_dp(m, scrape_time))
                .collect();
            if dps.is_empty() {
                return None;
            }
            (
                MetricType::ExponentialHistogram,
                DataPointsOwned::ExponentialHistogram(ExponentialHistogramOwned {
                    data_points: dps,
                    aggregation_temporality: AggregationTemporality::Cumulative,
                }),
            )
        }
        T::Info => {
            let dps: Vec<_> = metric
                .into_iter()
                .filter_map(|m| info_dp(m, scrape_time))
                .collect();
            if dps.is_empty() {
                return None;
            }
            (
                MetricType::Sum,
                DataPointsOwned::Sum(SumOwned {
                    data_points: dps,
                    aggregation_temporality: AggregationTemporality::Cumulative,
                    is_monotonic: false,
                }),
            )
        }
        T::StateSet => {
            let mut dps = Vec::new();
            for m in metric {
                stateset_dps(m, &name, scrape_time, &mut dps);
            }
            if dps.is_empty() {
                return None;
            }
            (
                MetricType::Sum,
                DataPointsOwned::Sum(SumOwned {
                    data_points: dps,
                    aggregation_temporality: AggregationTemporality::Cumulative,
                    is_monotonic: false,
                }),
            )
        }
        // Gauge histograms have no OTLP equivalent and MUST be dropped per spec.
        T::GaugeHistogram => {
            debug!(metric = %name, "dropping gauge histogram (no OTLP equivalent)");
            return None;
        }
    };

    Some(MetricRowOwned {
        name: ZStringOwned::String(name),
        r#type: otlp_type,
        description: help.map(ZStringOwned::String),
        unit: unit.map(ZStringOwned::String),
        source_ip: None,
        source_port: None,
        data_points,
        attrs: Vec::new(),
    })
}

fn counter_dp(m: owned::Metric, scrape_time: DateTime<Utc>) -> Option<NumberDataPointOwned> {
    let owned::MetricValue::Counter(c) = m.value else {
        return None;
    };
    Some(NumberDataPointOwned {
        start_time: c.created_timestamp,
        time: m.timestamp.unwrap_or(scrape_time),
        value: unsigned_to_value(c.value),
        flags: 0,
        attrs: labels_to_attrs(m.label),
        exemplars: c
            .exemplar
            .into_iter()
            .map(|e| conv_exemplar(e, scrape_time))
            .collect(),
    })
}

fn gauge_dp(m: owned::Metric, scrape_time: DateTime<Utc>) -> Option<NumberDataPointOwned> {
    let n = match m.value {
        owned::MetricValue::Gauge(n) | owned::MetricValue::Untyped(n) => n,
        _ => return None,
    };
    Some(NumberDataPointOwned {
        start_time: None,
        time: m.timestamp.unwrap_or(scrape_time),
        value: number_to_value(n),
        flags: 0,
        attrs: labels_to_attrs(m.label),
        exemplars: Vec::new(),
    })
}

fn summary_dp(m: owned::Metric, scrape_time: DateTime<Utc>) -> Option<SummaryDataPointOwned> {
    let owned::MetricValue::Summary(s) = m.value else {
        return None;
    };
    Some(SummaryDataPointOwned {
        start_time: s.created_timestamp,
        time: m.timestamp.unwrap_or(scrape_time),
        count: s.sample_count.unwrap_or(0),
        sum: s.sample_sum.map(|n| n.as_f64()).unwrap_or(0.0),
        quantiles: s
            .quantile
            .into_iter()
            .map(|q| Quantile {
                quantile: q.quantile,
                value: q.value,
            })
            .collect(),
        flags: 0,
        attrs: labels_to_attrs(m.label),
    })
}

fn histogram_dp(m: owned::Metric, scrape_time: DateTime<Utc>) -> Option<HistogramDataPointOwned> {
    let owned::MetricValue::Histogram(h) = m.value else {
        return None;
    };
    let (explicit_bounds, bucket_counts, count, exemplars) = decumulate(h.counts, scrape_time);
    Some(HistogramDataPointOwned {
        start_time: h.created_timestamp,
        time: m.timestamp.unwrap_or(scrape_time),
        count,
        sum: h.sample_sum.map(|n| n.as_f64()),
        bucket_counts,
        explicit_bounds,
        flags: 0,
        min: None,
        max: None,
        attrs: labels_to_attrs(m.label),
        exemplars,
    })
}

fn exp_histogram_dp(
    m: owned::Metric,
    scrape_time: DateTime<Utc>,
) -> Option<ExponentialHistogramDataPointOwned> {
    let nh = match m.value {
        owned::MetricValue::NativeHistogram(nh) => nh,
        // For a hybrid (classic + native) histogram we keep only the native part,
        // matching the collector-contrib behaviour.
        owned::MetricValue::HybridHistogram { native, .. } => native,
        _ => return None,
    };
    // OTLP exponential histograms only express the standard exponential schemas.
    // Custom-bucket native histograms (schema -53) and anything out of range can't
    // be represented (the scraper doesn't expose their explicit bounds either).
    if !(-4..=8).contains(&nh.schema) {
        debug!(
            schema = nh.schema,
            "dropping native histogram: unsupported schema"
        );
        return None;
    }

    let (count, zero_count, positive, negative) = match nh.counts {
        owned::NativeCounts::Int {
            sample_count,
            zero_count,
            positive_spans,
            positive_deltas,
            negative_spans,
            negative_deltas,
        } => {
            let positive = convert_delta_buckets(&positive_spans, &positive_deltas);
            let negative = convert_delta_buckets(&negative_spans, &negative_deltas);
            let count =
                sample_count.unwrap_or_else(|| native_total(zero_count, &positive, &negative));
            (count, zero_count, positive, negative)
        }
        owned::NativeCounts::Float {
            sample_count,
            zero_count,
            positive_spans,
            positive_counts,
            negative_spans,
            negative_counts,
        } => {
            let positive = convert_absolute_buckets(&positive_spans, &positive_counts);
            let negative = convert_absolute_buckets(&negative_spans, &negative_counts);
            let zero_count = zero_count.round().max(0.0) as u64;
            let count = sample_count
                .map(|f| f.round().max(0.0) as u64)
                .unwrap_or_else(|| native_total(zero_count, &positive, &negative));
            (count, zero_count, positive, negative)
        }
    };

    Some(ExponentialHistogramDataPointOwned {
        start_time: nh.created_timestamp,
        time: m.timestamp.unwrap_or(scrape_time),
        count,
        sum: nh.sample_sum.map(|n| n.as_f64()),
        scale: nh.schema,
        zero_count,
        positive,
        negative,
        zero_threshold: nh.zero_threshold,
        min: None,
        max: None,
        flags: 0,
        attrs: labels_to_attrs(m.label),
        exemplars: nh
            .exemplars
            .into_iter()
            .map(|e| conv_exemplar(e, scrape_time))
            .collect(),
    })
}

fn info_dp(m: owned::Metric, scrape_time: DateTime<Utc>) -> Option<NumberDataPointOwned> {
    let owned::MetricValue::Info(info) = m.value else {
        return None;
    };
    let mut attrs = labels_to_attrs(m.label);
    attrs.extend(labels_to_attrs(info.labels));
    Some(NumberDataPointOwned {
        start_time: None,
        time: m.timestamp.unwrap_or(scrape_time),
        value: NumberValue::Int(1),
        flags: 0,
        attrs,
        exemplars: Vec::new(),
    })
}

/// A StateSet expands to one non-monotonic Sum data point per state, carrying the
/// state name under a label keyed by the metric family name (OpenMetrics convention)
/// and a 0/1 value.
fn stateset_dps(
    m: owned::Metric,
    family_name: &str,
    scrape_time: DateTime<Utc>,
    out: &mut Vec<NumberDataPointOwned>,
) {
    let owned::MetricValue::StateSet(ss) = m.value else {
        return;
    };
    let base_attrs = labels_to_attrs(m.label);
    let time = m.timestamp.unwrap_or(scrape_time);
    for state in ss.states {
        let mut attrs = base_attrs.clone();
        attrs.push((
            ZStringOwned::String(family_name.to_string()),
            ValueOwned::String(ZStringOwned::String(state.name)),
        ));
        out.push(NumberDataPointOwned {
            start_time: None,
            time,
            value: NumberValue::Int(if state.enabled { 1 } else { 0 }),
            flags: 0,
            attrs,
            exemplars: Vec::new(),
        });
    }
}

// ---- conversion helpers ---------------------------------------------------

fn labels_to_attrs(labels: Vec<owned::LabelPair>) -> Vec<(ZStringOwned, ValueOwned)> {
    labels
        .into_iter()
        .map(|l| {
            (
                ZStringOwned::String(l.name),
                ValueOwned::String(ZStringOwned::String(l.value)),
            )
        })
        .collect()
}

fn number_to_value(n: owned::Number) -> NumberValue {
    match n {
        owned::Number::Int(i) => NumberValue::Int(i),
        owned::Number::Float(f) => NumberValue::Float(f),
    }
}

fn unsigned_to_value(n: owned::UnsignedNumber) -> NumberValue {
    match n {
        owned::UnsignedNumber::Uint(u) if u <= i64::MAX as u64 => NumberValue::Int(u as i64),
        owned::UnsignedNumber::Uint(u) => NumberValue::Float(u as f64),
        owned::UnsignedNumber::Float(f) => NumberValue::Float(f),
    }
}

fn conv_exemplar(e: owned::Exemplar, scrape_time: DateTime<Utc>) -> ExemplarOwned {
    let mut trace_id = None;
    let mut span_id = None;
    let mut attrs = Vec::new();
    for l in e.label {
        match l.name.as_str() {
            "trace_id" => trace_id = parse_hex::<16>(&l.value),
            "span_id" => span_id = parse_hex::<8>(&l.value),
            _ => attrs.push((
                ZStringOwned::String(l.name),
                ValueOwned::String(ZStringOwned::String(l.value)),
            )),
        }
    }
    ExemplarOwned {
        time: e.timestamp.unwrap_or(scrape_time),
        value: NumberValue::Float(e.value),
        span_id,
        trace_id,
        attrs,
    }
}

fn parse_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Convert a classic Prometheus histogram's cumulative buckets into OTLP's
/// per-bucket layout: drop the implicit `+Inf` bound, de-cumulate counts, and carry
/// the `+Inf` overflow as the final bucket count. Returns
/// `(explicit_bounds, bucket_counts, total_count, exemplars)`.
fn decumulate(
    counts: owned::BucketCount,
    scrape_time: DateTime<Utc>,
) -> (Vec<f64>, Vec<u64>, u64, Vec<ExemplarOwned>) {
    let mut bounds = Vec::new();
    let mut bucket_counts = Vec::new();
    let mut exemplars = Vec::new();

    let total = match counts {
        owned::BucketCount::Int {
            sample_count,
            mut buckets,
        } => {
            buckets.sort_by(|a, b| a.upper_bound.total_cmp(&b.upper_bound));
            let mut prev = 0u64;
            for b in buckets {
                let bucket = b.cumulative_count.saturating_sub(prev);
                prev = b.cumulative_count;
                if b.upper_bound.is_finite() {
                    bounds.push(b.upper_bound);
                }
                bucket_counts.push(bucket);
                if let Some(ex) = b.exemplar {
                    exemplars.push(conv_exemplar(ex, scrape_time));
                }
            }
            sample_count.unwrap_or(prev)
        }
        owned::BucketCount::Float {
            sample_count,
            mut buckets,
        } => {
            buckets.sort_by(|a, b| a.upper_bound.total_cmp(&b.upper_bound));
            let mut prev = 0.0f64;
            for b in buckets {
                let bucket = (b.cumulative_count - prev).round().max(0.0) as u64;
                prev = b.cumulative_count;
                if b.upper_bound.is_finite() {
                    bounds.push(b.upper_bound);
                }
                bucket_counts.push(bucket);
                if let Some(ex) = b.exemplar {
                    exemplars.push(conv_exemplar(ex, scrape_time));
                }
            }
            sample_count
                .map(|f| f.round().max(0.0) as u64)
                .unwrap_or(prev as u64)
        }
    };

    // OTLP requires bucket_counts.len() == explicit_bounds.len() + 1. If the source
    // had no explicit `+Inf` bucket, add the (empty) overflow bucket.
    if bucket_counts.len() == bounds.len() {
        bucket_counts.push(0);
    }

    (bounds, bucket_counts, total, exemplars)
}

/// Decode delta-encoded integer native-histogram buckets (one side) into OTLP's
/// dense `(offset, bucket_counts)`. Ported from collector-contrib's
/// `convertDeltaBuckets`: the first span's offset folds into the OTLP offset (minus
/// one, for the Prometheus->OTLP bucket-index shift), later spans zero-fill their gap.
fn convert_delta_buckets(spans: &[owned::BucketSpan], deltas: &[i64]) -> Option<BucketsOwned> {
    let first = spans.first()?;
    let mut bucket_counts = Vec::with_capacity(deltas.len());
    let mut idx = 0usize;
    let mut running = 0i64;
    for (span_idx, span) in spans.iter().enumerate() {
        if span_idx > 0 {
            bucket_counts.extend(std::iter::repeat_n(0u64, span.offset.max(0) as usize));
        }
        for _ in 0..span.length {
            let Some(delta) = deltas.get(idx) else { break };
            running += delta;
            idx += 1;
            bucket_counts.push(running.max(0) as u64);
        }
    }
    Some(BucketsOwned {
        offset: first.offset - 1,
        bucket_counts,
    })
}

/// Like [`convert_delta_buckets`] but for float native histograms, whose per-bucket
/// counts are absolute (not delta-encoded). Counts are rounded to `u64`.
fn convert_absolute_buckets(spans: &[owned::BucketSpan], counts: &[f64]) -> Option<BucketsOwned> {
    let first = spans.first()?;
    let mut bucket_counts = Vec::with_capacity(counts.len());
    let mut idx = 0usize;
    for (span_idx, span) in spans.iter().enumerate() {
        if span_idx > 0 {
            bucket_counts.extend(std::iter::repeat_n(0u64, span.offset.max(0) as usize));
        }
        for _ in 0..span.length {
            let Some(count) = counts.get(idx) else { break };
            idx += 1;
            bucket_counts.push(count.round().max(0.0) as u64);
        }
    }
    Some(BucketsOwned {
        offset: first.offset - 1,
        bucket_counts,
    })
}

/// Fallback total for a native histogram with no explicit sample count: the zero
/// bucket plus every populated positive/negative bucket.
fn native_total(
    zero_count: u64,
    positive: &Option<BucketsOwned>,
    negative: &Option<BucketsOwned>,
) -> u64 {
    let sum_side = |b: &Option<BucketsOwned>| {
        b.as_ref()
            .map(|b| b.bucket_counts.iter().sum::<u64>())
            .unwrap_or(0)
    };
    zero_count + sum_side(positive) + sum_side(negative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus_scraper::owned;

    fn bi(cumulative_count: u64, upper_bound: f64) -> owned::BucketInt {
        owned::BucketInt {
            cumulative_count,
            upper_bound,
            exemplar: None,
        }
    }

    fn label(name: &str, value: &str) -> owned::LabelPair {
        owned::LabelPair {
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    fn span(offset: i32, length: u32) -> owned::BucketSpan {
        owned::BucketSpan { offset, length }
    }

    #[test]
    fn decumulate_classic_int_histogram() {
        // Cumulative buckets: le=1 -> 2, le=5 -> 5, le=+Inf -> 9.
        let counts = owned::BucketCount::Int {
            sample_count: Some(9),
            buckets: vec![bi(2, 1.0), bi(5, 5.0), bi(9, f64::INFINITY)],
        };
        let (bounds, bucket_counts, total, exemplars) = decumulate(counts, Utc::now());
        assert_eq!(bounds, vec![1.0, 5.0]); // +Inf excluded
        assert_eq!(bucket_counts, vec![2, 3, 4]); // de-cumulated, last is +Inf overflow
        assert_eq!(total, 9);
        assert_eq!(bucket_counts.len(), bounds.len() + 1);
        assert!(exemplars.is_empty());
    }

    #[test]
    fn decumulate_sorts_and_handles_missing_inf() {
        // Out of order and no +Inf bucket present.
        let counts = owned::BucketCount::Int {
            sample_count: None,
            buckets: vec![bi(7, 10.0), bi(3, 2.0)],
        };
        let (bounds, bucket_counts, total, _) = decumulate(counts, Utc::now());
        assert_eq!(bounds, vec![2.0, 10.0]);
        assert_eq!(bucket_counts, vec![3, 4, 0]); // synthetic empty overflow bucket
        assert_eq!(total, 7); // falls back to the highest cumulative count
    }

    #[test]
    fn convert_delta_buckets_offset_and_gap() {
        // Two spans: first at offset 2 (len 2), second at gap 1 (len 1).
        // Deltas [1, 2, -1] -> running counts 1, 3, 2.
        let spans = vec![span(2, 2), span(1, 1)];
        let deltas = vec![1i64, 2, -1];
        let buckets = convert_delta_buckets(&spans, &deltas).unwrap();
        assert_eq!(buckets.offset, 1); // first.offset - 1
        // first span: [1, 3]; gap of 1 zero; second span: [2]
        assert_eq!(buckets.bucket_counts, vec![1, 3, 0, 2]);
    }

    #[test]
    fn convert_absolute_buckets_rounds() {
        let spans = vec![span(0, 2)];
        let counts = vec![1.4f64, 2.6];
        let buckets = convert_absolute_buckets(&spans, &counts).unwrap();
        assert_eq!(buckets.offset, -1);
        assert_eq!(buckets.bucket_counts, vec![1, 3]);
    }

    #[test]
    fn empty_spans_yield_no_buckets() {
        assert!(convert_delta_buckets(&[], &[]).is_none());
        assert!(convert_absolute_buckets(&[], &[]).is_none());
    }

    #[test]
    fn counter_family_becomes_monotonic_sum() {
        let family = owned::MetricFamily {
            name: "http_requests".to_string(),
            help: Some("total requests".to_string()),
            r#type: owned::MetricType::Counter,
            unit: None,
            metric: vec![owned::Metric {
                label: vec![label("method", "GET")],
                value: owned::MetricValue::Counter(owned::Counter {
                    value: owned::UnsignedNumber::Uint(7),
                    exemplar: None,
                    created_timestamp: None,
                }),
                timestamp: None,
            }],
        };
        let row = family_to_row(family, Utc::now()).unwrap();
        assert_eq!(row.r#type, MetricType::Sum);
        match row.data_points {
            DataPointsOwned::Sum(s) => {
                assert!(s.is_monotonic);
                assert_eq!(
                    s.aggregation_temporality,
                    AggregationTemporality::Cumulative
                );
                assert_eq!(s.data_points.len(), 1);
                assert!(matches!(s.data_points[0].value, NumberValue::Int(7)));
                assert_eq!(s.data_points[0].attrs.len(), 1);
            }
            _ => panic!("expected Sum"),
        }
    }

    #[test]
    fn gauge_family_becomes_gauge() {
        let family = owned::MetricFamily {
            name: "temp".to_string(),
            help: None,
            r#type: owned::MetricType::Gauge,
            unit: None,
            metric: vec![owned::Metric {
                label: vec![],
                value: owned::MetricValue::Gauge(owned::Number::Float(21.5)),
                timestamp: None,
            }],
        };
        let row = family_to_row(family, Utc::now()).unwrap();
        assert_eq!(row.r#type, MetricType::Gauge);
        assert!(matches!(row.data_points, DataPointsOwned::Gauge(_)));
    }

    #[test]
    fn stateset_expands_to_non_monotonic_sum_per_state() {
        let family = owned::MetricFamily {
            name: "feature".to_string(),
            help: None,
            r#type: owned::MetricType::StateSet,
            unit: None,
            metric: vec![owned::Metric {
                label: vec![],
                value: owned::MetricValue::StateSet(owned::StateSet {
                    states: vec![
                        owned::State {
                            name: "a".to_string(),
                            enabled: true,
                        },
                        owned::State {
                            name: "b".to_string(),
                            enabled: false,
                        },
                    ],
                }),
                timestamp: None,
            }],
        };
        let row = family_to_row(family, Utc::now()).unwrap();
        match row.data_points {
            DataPointsOwned::Sum(s) => {
                assert!(!s.is_monotonic);
                assert_eq!(s.data_points.len(), 2);
                assert!(matches!(s.data_points[0].value, NumberValue::Int(1)));
                assert!(matches!(s.data_points[1].value, NumberValue::Int(0)));
                // each carries the family-name -> state-name attribute
                assert_eq!(s.data_points[0].attrs.len(), 1);
            }
            _ => panic!("expected Sum"),
        }
    }

    #[test]
    fn gauge_histogram_is_dropped() {
        let family = owned::MetricFamily {
            name: "gh".to_string(),
            help: None,
            r#type: owned::MetricType::GaugeHistogram,
            unit: None,
            metric: vec![owned::Metric {
                label: vec![],
                value: owned::MetricValue::GaugeHistogram(owned::Histogram {
                    sample_sum: None,
                    counts: owned::BucketCount::Int {
                        sample_count: Some(0),
                        buckets: vec![bi(0, f64::INFINITY)],
                    },
                    created_timestamp: None,
                }),
                timestamp: None,
            }],
        };
        assert!(family_to_row(family, Utc::now()).is_none());
    }
}
