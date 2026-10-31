// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Stable table names for OTAP payload types.
//!
//! OTAP payload types are an enum of wire-level record kinds; this maps the ones
//! we surface to a short, stable name. Used by `Payload`'s `Debug` impl to label
//! each record batch by name rather than by payload-type discriminant.

use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;

/// Stable table name for an OTAP payload type. Returns `None` for payload
/// types we do not expose (those without a schema, like `Unknown` and
/// `MultivariateMetrics`).
pub(super) fn table_name(pt: ArrowPayloadType) -> Option<&'static str> {
    use ArrowPayloadType as P;
    Some(match pt {
        // Logs
        P::Logs => "logs",
        P::LogAttrs => "log_attrs",
        // Common
        P::ResourceAttrs => "resource_attrs",
        P::ScopeAttrs => "scope_attrs",
        // Traces
        P::Spans => "spans",
        P::SpanAttrs => "span_attrs",
        P::SpanEvents => "span_events",
        P::SpanLinks => "span_links",
        P::SpanEventAttrs => "span_event_attrs",
        P::SpanLinkAttrs => "span_link_attrs",
        // Metrics
        P::UnivariateMetrics => "metrics",
        P::MetricAttrs => "metric_attrs",
        P::NumberDataPoints => "number_data_points",
        P::SummaryDataPoints => "summary_data_points",
        P::HistogramDataPoints => "histogram_data_points",
        P::ExpHistogramDataPoints => "exp_histogram_data_points",
        P::NumberDpAttrs => "number_dp_attrs",
        P::SummaryDpAttrs => "summary_dp_attrs",
        P::HistogramDpAttrs => "histogram_dp_attrs",
        P::ExpHistogramDpAttrs => "exp_histogram_dp_attrs",
        P::NumberDpExemplars => "number_dp_exemplars",
        P::HistogramDpExemplars => "histogram_dp_exemplars",
        P::ExpHistogramDpExemplars => "exp_histogram_dp_exemplars",
        P::NumberDpExemplarAttrs => "number_dp_exemplar_attrs",
        P::HistogramDpExemplarAttrs => "histogram_dp_exemplar_attrs",
        P::ExpHistogramDpExemplarAttrs => "exp_histogram_dp_exemplar_attrs",
        // No schema / not exposed.
        _ => return None,
    })
}
