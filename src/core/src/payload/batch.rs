// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::attr::{self, AttrLocation};
use super::{Payload, ZStringOwned};
use anyhow::{Context, Result};
use arrow::array::{Array, AsArray, BooleanArray};
use arrow::datatypes::UInt16Type;
use otel_arrow_dfe_config::SignalType;
use otel_arrow_dfe_pdata::OtapArrowRecords;
use otel_arrow_dfe_pdata::otap::batching::make_item_batches;
use otel_arrow_dfe_pdata::otap::filter::{IdBitmapPool, filter_otap_batch};
use otel_arrow_dfe_pdata::proto::opentelemetry::arrow::v1::ArrowPayloadType;
use otel_arrow_dfe_pdata::schema::consts;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use std::num::NonZeroU64;

impl Payload {
    /// Split the payload into a piece holding (about) `mid` items and a piece
    /// holding the rest, returned as `(mid_sized, rest)`. Either element is
    /// `None` when it would be empty: `mid == 0` gives `(None, Some(_))` and
    /// `mid >= self.num_items()` gives `(Some(_), None)`.
    ///
    /// Unlike [`slice::split_at`], this is **not positional** — it guarantees
    /// the item *count* of the first element (`≈ mid`), not that it holds the
    /// first `mid` rows. It's built on [`make_item_batches`] (there's no
    /// lower-level split exposed for `OtapArrowRecords`), and to do the split
    /// in a single pass it cuts at `max(mid, len - mid)`: that limit is always
    /// `≥ len/2`, so the batcher yields exactly two chunks (`2 * limit >= len`,
    /// no third chunk) while still splitting (`limit < len`) — no separate
    /// merge pass, and both pieces come out as zero-copy Arrow slices.
    ///
    /// The leading chunk holds `limit` items, so when `mid >= len/2` the
    /// `mid`-sized piece is the front and global row order is preserved. When
    /// `mid < len/2` the `mid`-sized piece is the *trailing* chunk, so the two
    /// returned pieces appear swapped relative to the input (each piece is
    /// still internally ordered). A consequence: `split_items(k)` and
    /// `split_items(len - k)` return the same partition with the elements
    /// swapped.
    ///
    /// For logs/traces an item is a row, so the counts are exact. For metrics
    /// an item is a data point and a single metric row can't be split mid-row,
    /// so the first element may hold slightly more or fewer than `mid` items
    /// (and, rarely, a data-point-heavy row forces a small merge of the
    /// remainder).
    pub fn split_items(&self, mid: usize) -> Result<(Option<Payload>, Option<Payload>)> {
        if mid == 0 {
            return Ok((None, Some(self.clone())));
        }
        let len = self.num_items();
        if mid >= len {
            return Ok((Some(self.clone()), None));
        }

        let signal = match &self.data {
            OtapArrowRecords::Logs(_) => SignalType::Logs,
            OtapArrowRecords::Metrics(_) => SignalType::Metrics,
            OtapArrowRecords::Traces(_) => SignalType::Traces,
        };
        // Cut at max(mid, len - mid). The leading chunk holds `limit` items, so
        // the `mid`-sized piece is the front when mid >= len/2, else the tail.
        let mid_in_front = mid >= len - mid;
        let boundary = if mid_in_front { mid } else { len - mid };
        let limit = NonZeroU64::try_from(boundary as u64).unwrap();

        let mut chunks = make_item_batches(signal, Some(limit), vec![self.data.clone()])
            .context("split_items")?;
        let head = chunks.remove(0);
        // Exactly two chunks in the common case; a data-point-heavy metric row
        // can rarely force more, so merge any trailing chunks back into one.
        let tail = match chunks.len() {
            0 => None,
            1 => chunks.pop(),
            _ => make_item_batches(signal, None, chunks)
                .context("split_items merge remainder")?
                .pop(),
        };

        // Orient the tuple so `.0` is the `mid`-sized piece.
        let (first, rest) = match (mid_in_front, tail) {
            (true, tail) => (Some(head), tail),
            (false, Some(tail)) => (Some(tail), Some(head)),
            // Indivisible (e.g. a single oversized metric row): hand it back whole.
            (false, None) => (Some(head), None),
        };
        Ok((first.map(Payload::new), rest.map(Payload::new)))
    }
}

#[derive(Default)]
pub struct PayloadBatch {
    logs: Vec<Payload>,
    metrics: Vec<Payload>,
    traces: Vec<Payload>,
}

impl PayloadBatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, payload: Payload) {
        match payload.data {
            OtapArrowRecords::Logs(_) => self.logs.push(payload),
            OtapArrowRecords::Metrics(_) => self.metrics.push(payload),
            OtapArrowRecords::Traces(_) => self.traces.push(payload),
        }
    }

    pub fn extend(&mut self, other: Self) {
        self.logs.extend(other.logs);
        self.metrics.extend(other.metrics);
        self.traces.extend(other.traces);
    }

    /// Total [`Payload::num_items`] across the batch — data points for metrics,
    /// rows for logs and traces. This is the unit `batch_size` is expressed in
    /// and the one [`Self::batch`] splits on.
    pub fn num_items(&self) -> usize {
        self.iter().map(|p| p.num_items()).sum()
    }

    /// Total [`Payload::num_rows`] across the batch — root records, i.e. log
    /// records / metrics / spans. Differs from [`Self::num_items`] for metrics,
    /// where one row can carry any number of data points (including none).
    ///
    /// Not cached: this walks the payloads, reading one length off each root
    /// record batch.
    pub fn num_rows(&self) -> usize {
        self.iter().map(|p| p.num_rows()).sum()
    }

    /// Whether the batch holds no payloads at all.
    ///
    /// Distinct from `num_items() == 0`, which is also true for a batch holding
    /// only metrics rows that carry no data points. Flush gates want this one so
    /// such rows are not stranded in the buffer.
    pub fn is_empty(&self) -> bool {
        self.logs.is_empty() && self.metrics.is_empty() && self.traces.is_empty()
    }

    /// Number of [`Payload`]s held (not records).
    pub fn num_payloads(&self) -> usize {
        self.logs.len() + self.metrics.len() + self.traces.len()
    }

    pub fn batch(&mut self, num: Option<usize>) -> Self {
        let num = match num {
            None => Some(NonZeroU64::try_from(u16::MAX as u64).unwrap()),
            Some(num) => {
                if num >= u16::MAX as usize {
                    Some(NonZeroU64::try_from(u16::MAX as u64).unwrap())
                } else {
                    Some(NonZeroU64::try_from(num as u64).unwrap())
                }
            }
        };
        let logs: Vec<_> = self.logs.drain(..).map(|p| p.data).collect();
        let metrics: Vec<_> = self.metrics.drain(..).map(|p| p.data).collect();
        let traces: Vec<_> = self.traces.drain(..).map(|p| p.data).collect();

        let logs = make_item_batches(SignalType::Logs, num, logs).unwrap();
        let metrics = make_item_batches(SignalType::Metrics, num, metrics).unwrap();
        let traces = make_item_batches(SignalType::Traces, num, traces).unwrap();

        let logs: Vec<_> = logs.into_iter().map(Payload::new).collect();
        let metrics: Vec<_> = metrics.into_iter().map(Payload::new).collect();
        let traces: Vec<_> = traces.into_iter().map(Payload::new).collect();

        Self {
            logs,
            metrics,
            traces,
        }
    }

    /// Split the batch by the value of one attribute — a `GROUP BY` over an
    /// attribute column. `location` selects which attribute table to read
    /// (record / scope / resource) and `key` the attribute name within it.
    ///
    /// Every record in the batch is assigned a group key: `Some(value)` when it
    /// carries `key` with a `str` or `int` value (ints rendered as their decimal
    /// string), or `None` when the attribute is absent or of any other type. Each
    /// returned group is a [`PayloadBatch`] holding the matching records, merged
    /// so each signal type collapses to a single [`Payload`] (subject to the same
    /// `u16::MAX` items-per-batch cap as [`Self::batch`]).
    ///
    /// The output order is unspecified.
    pub fn shard(
        &self,
        location: AttrLocation,
        key: &str,
    ) -> Result<Vec<(Option<ZStringOwned>, PayloadBatch)>> {
        let mut pool = IdBitmapPool::new();
        // value -> (logs, metrics, traces) sub-batches awaiting merge.
        type Signals = (
            Vec<OtapArrowRecords>,
            Vec<OtapArrowRecords>,
            Vec<OtapArrowRecords>,
        );
        let mut groups: FxHashMap<Option<SmolStr>, Signals> = FxHashMap::default();
        for pl in self.iter() {
            for (val, sub) in shard_one(pl, location, key, &mut pool)? {
                let entry = groups.entry(val).or_default();
                match sub {
                    OtapArrowRecords::Logs(_) => entry.0.push(sub),
                    OtapArrowRecords::Metrics(_) => entry.1.push(sub),
                    OtapArrowRecords::Traces(_) => entry.2.push(sub),
                }
            }
        }

        let num = Some(NonZeroU64::try_from(u16::MAX as u64).unwrap());
        let mut out = Vec::with_capacity(groups.len());
        for (val, (logs, metrics, traces)) in groups {
            let mut batch = PayloadBatch::new();
            for (signal, parts) in [
                (SignalType::Logs, logs),
                (SignalType::Metrics, metrics),
                (SignalType::Traces, traces),
            ] {
                if parts.is_empty() {
                    continue;
                }
                let merged = make_item_batches(signal, num, parts)
                    .map_err(|e| anyhow::anyhow!("shard merge {signal:?}: {e}"))?;
                for b in merged {
                    batch.push(Payload::new(b));
                }
            }
            out.push((val.map(ZStringOwned::from), batch));
        }
        Ok(out)
    }

    /// Iterate over every [`Payload`] in the batch, in `logs`, `metrics`, then
    /// `traces` order.
    pub fn iter(&self) -> impl Iterator<Item = &Payload> {
        self.logs
            .iter()
            .chain(self.metrics.iter())
            .chain(self.traces.iter())
    }

    /// Mutably iterate over every [`Payload`] in the batch.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Payload> {
        self.logs
            .iter_mut()
            .chain(self.metrics.iter_mut())
            .chain(self.traces.iter_mut())
    }
}

/// Shard a single payload: assign each root row a group key (`None` when the
/// attribute is absent / non-str-int) and `filter_otap_batch` it into one
/// sub-record-set per distinct value, preserving child-table relationships.
fn shard_one(
    pl: &Payload,
    loc: AttrLocation,
    key: &str,
    pool: &mut IdBitmapPool,
) -> Result<Vec<(Option<SmolStr>, OtapArrowRecords)>> {
    // Guard on root rows, not items: a metrics payload whose rows carry no data
    // points has `num_items() == 0` but still holds records we must not drop.
    if pl.num_rows() == 0 {
        return Ok(vec![]);
    }
    let attr_batch = match loc {
        AttrLocation::Record => attr::get_record_attr(pl),
        AttrLocation::Resource => pl.data.get(ArrowPayloadType::ResourceAttrs),
        AttrLocation::Scope => pl.data.get(ArrowPayloadType::ScopeAttrs),
    };
    let pid2val = match attr_batch {
        Some(b) => attr::parent_id_values(b, key)?,
        None => FxHashMap::default(),
    };
    // Without a root record there is nothing to mask — pass the whole payload
    // through under the `None` group.
    let Some(record) = attr::get_record(pl) else {
        return Ok(vec![(None, pl.data.clone())]);
    };
    let n = record.num_rows();
    let id_col = match loc {
        AttrLocation::Record => record.column_by_name(consts::ID),
        AttrLocation::Resource => attr::struct_id_column(record, consts::RESOURCE),
        AttrLocation::Scope => attr::struct_id_column(record, consts::SCOPE),
    };
    let id_col = id_col.and_then(|c| c.as_primitive_opt::<UInt16Type>());

    // Bucket row indices by their group value into per-value masks.
    let mut masks: FxHashMap<Option<SmolStr>, Vec<bool>> = FxHashMap::default();
    for i in 0..n {
        let val = match id_col {
            Some(c) if c.is_valid(i) => pid2val.get(&c.value(i)).cloned(),
            _ => None,
        };
        masks.entry(val).or_insert_with(|| vec![false; n])[i] = true;
    }

    let mut out = Vec::with_capacity(masks.len());
    for (val, mask) in masks {
        let sub = filter_otap_batch(&BooleanArray::from(mask), &pl.data, pool)
            .map_err(|e| anyhow::anyhow!("shard filter: {e}"))?;
        out.push((val, sub));
    }
    Ok(out)
}

impl IntoIterator for PayloadBatch {
    type Item = Payload;
    type IntoIter = std::iter::Chain<
        std::iter::Chain<std::vec::IntoIter<Payload>, std::vec::IntoIter<Payload>>,
        std::vec::IntoIter<Payload>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.logs.into_iter().chain(self.metrics).chain(self.traces)
    }
}

impl<'a> IntoIterator for &'a PayloadBatch {
    type Item = &'a Payload;
    type IntoIter = std::iter::Chain<
        std::iter::Chain<std::slice::Iter<'a, Payload>, std::slice::Iter<'a, Payload>>,
        std::slice::Iter<'a, Payload>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.logs
            .iter()
            .chain(self.metrics.iter())
            .chain(self.traces.iter())
    }
}

impl<'a> IntoIterator for &'a mut PayloadBatch {
    type Item = &'a mut Payload;
    type IntoIter = std::iter::Chain<
        std::iter::Chain<std::slice::IterMut<'a, Payload>, std::slice::IterMut<'a, Payload>>,
        std::slice::IterMut<'a, Payload>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.logs
            .iter_mut()
            .chain(self.metrics.iter_mut())
            .chain(self.traces.iter_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::PayloadBatch;
    use crate::config::Format;
    use crate::payload::{AttrLocation, Payload, ZStringOwned, attr};
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::encode::encode_metrics_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::any_value::Value;
    use otel_arrow_dfe_pdata::proto::opentelemetry::common::v1::{
        AnyValue, InstrumentationScope, KeyValue,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::metrics::v1::{
        Gauge, Metric, MetricsData, NumberDataPoint, ResourceMetrics as ProtoResourceMetrics,
        ScopeMetrics as ProtoScopeMetrics, metric, number_data_point,
    };
    use otel_arrow_dfe_pdata::proto::opentelemetry::resource::v1::Resource;

    fn kv(key: &str, value: Value) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue { value: Some(value) }),
        }
    }

    fn record(attrs: Vec<KeyValue>) -> LogRecord {
        LogRecord {
            time_unix_nano: 1,
            attributes: attrs,
            ..Default::default()
        }
    }

    fn resource_logs(res_attrs: Vec<KeyValue>, records: Vec<LogRecord>) -> ResourceLogs {
        ResourceLogs {
            resource: Some(Resource {
                attributes: res_attrs,
                ..Default::default()
            }),
            scope_logs: vec![ScopeLogs {
                scope: Some(InstrumentationScope {
                    name: "test-scope".to_string(),
                    version: "1.0".to_string(),
                    attributes: vec![],
                    dropped_attributes_count: 0,
                }),
                log_records: records,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }
    }

    fn logs_payload(resource_logs: Vec<ResourceLogs>) -> Payload {
        Payload::new(encode_logs_otap_batch(&LogsData { resource_logs }).unwrap())
    }

    fn metrics_payload(data: MetricsData) -> Payload {
        Payload::new(encode_metrics_otap_batch(&data).unwrap())
    }

    /// Total item count under the group whose key matches `key`.
    fn group_items(
        groups: &[(Option<ZStringOwned>, PayloadBatch)],
        key: Option<&str>,
    ) -> Option<usize> {
        groups
            .iter()
            .find(|(k, _)| k.as_ref().map(|z| z.as_ref()) == key)
            .map(|(_, b)| b.iter().map(|p| p.num_items()).sum::<usize>())
    }

    #[test]
    fn shard_by_resource_attribute() {
        // Two resources, host.name = "a" (2 records) / "b" (1 record).
        let pl = logs_payload(vec![
            resource_logs(
                vec![kv("host.name", Value::StringValue("a".into()))],
                vec![record(vec![]), record(vec![])],
            ),
            resource_logs(
                vec![kv("host.name", Value::StringValue("b".into()))],
                vec![record(vec![])],
            ),
        ]);
        let mut batch = PayloadBatch::new();
        batch.push(pl);

        let groups = batch.shard(AttrLocation::Resource, "host.name").unwrap();
        assert_eq!(groups.len(), 2, "expected one group per distinct value");
        assert_eq!(group_items(&groups, Some("a")), Some(2));
        assert_eq!(group_items(&groups, Some("b")), Some(1));
        // No records went missing.
        let total: usize = groups
            .iter()
            .map(|(_, b)| b.iter().map(|p| p.num_items()).sum::<usize>())
            .sum();
        assert_eq!(total, 3);
    }

    #[test]
    fn shard_by_record_int_with_missing() {
        // Record-level int attribute on two rows, absent on the third.
        let pl = logs_payload(vec![resource_logs(
            vec![],
            vec![
                record(vec![kv("shard.id", Value::IntValue(7))]),
                record(vec![kv("shard.id", Value::IntValue(7))]),
                record(vec![kv("other", Value::StringValue("x".into()))]),
            ],
        )]);
        let mut batch = PayloadBatch::new();
        batch.push(pl);

        let groups = batch.shard(AttrLocation::Record, "shard.id").unwrap();
        assert_eq!(groups.len(), 2);
        // int converted to its decimal string form.
        assert_eq!(group_items(&groups, Some("7")), Some(2));
        // missing attribute → None group.
        assert_eq!(group_items(&groups, None), Some(1));
    }

    #[test]
    fn shard_merges_across_payloads() {
        // Two separate logs payloads both carrying host.name = "a".
        let mut batch = PayloadBatch::new();
        batch.push(logs_payload(vec![resource_logs(
            vec![kv("host.name", Value::StringValue("a".into()))],
            vec![record(vec![])],
        )]));
        batch.push(logs_payload(vec![resource_logs(
            vec![kv("host.name", Value::StringValue("a".into()))],
            vec![record(vec![])],
        )]));

        let groups = batch.shard(AttrLocation::Resource, "host.name").unwrap();
        assert_eq!(groups.len(), 1);
        let (key, merged) = &groups[0];
        assert_eq!(key.as_ref().map(|z| z.as_ref()), Some("a"));
        // Merged into a single logs Payload holding both records.
        assert_eq!(
            merged.iter().count(),
            1,
            "same signal should merge to one Payload"
        );
        assert_eq!(merged.iter().map(|p| p.num_items()).sum::<usize>(), 2);
    }

    #[test]
    fn shard_on_unshardable_type_falls_in_none() {
        // A double-valued attribute can't be sharded → everything is `None`.
        let pl = logs_payload(vec![resource_logs(
            vec![kv("host.name", Value::DoubleValue(1.5))],
            vec![record(vec![])],
        )]);
        let mut batch = PayloadBatch::new();
        batch.push(pl);

        let groups = batch.shard(AttrLocation::Resource, "host.name").unwrap();
        assert_eq!(groups.len(), 1);
        assert!(groups[0].0.is_none(), "double value should not shard");
        assert_eq!(group_items(&groups, None), Some(1));
    }

    #[test]
    fn batch_drains_source() {
        let mut batch = PayloadBatch::new();
        batch.push(logs_payload(vec![resource_logs(
            vec![],
            vec![record(vec![])],
        )]));
        batch.push(logs_payload(vec![resource_logs(
            vec![],
            vec![record(vec![])],
        )]));
        assert_eq!(batch.num_items(), 2);

        let merged = batch.batch(None);
        assert_eq!(batch.num_items(), 0, "source must be empty after batch()");
        let merged: Vec<_> = merged.into_iter().collect();
        assert_eq!(merged.iter().map(|p| p.num_items()).sum::<usize>(), 2);

        // the drained batch is reusable
        batch.push(logs_payload(vec![resource_logs(
            vec![],
            vec![record(vec![])],
        )]));
        assert_eq!(batch.num_items(), 1);
    }

    /// `timeUnixNano` of every log record in a payload, in row order —
    /// extracted via the denormalized JSON serializer (one JSON doc per
    /// record), so it doubles as an ordering check independent of `split_items`.
    fn record_times(pl: &Payload) -> Vec<String> {
        pl.serialize_as(Format::JsonNestedDenormalized)
            .unwrap()
            .iter()
            .map(|b| {
                let v: serde_json::Value = serde_json::from_slice(b).unwrap();
                v["record"]["timeUnixNano"].as_str().unwrap().to_string()
            })
            .collect()
    }

    fn record_with_time(t: u64) -> LogRecord {
        LogRecord {
            time_unix_nano: t,
            ..Default::default()
        }
    }

    #[test]
    fn split_items_front_when_mid_is_larger_half() {
        // mid >= len/2 (10 of 15): the mid-sized piece is the leading chunk and
        // global row order is preserved.
        let records: Vec<_> = (1..=15).map(record_with_time).collect();
        let pl = logs_payload(vec![resource_logs(vec![], records)]);

        let (first, rest) = pl.split_items(10).unwrap();
        let first = first.expect("first half non-empty");
        let rest = rest.expect("remainder non-empty");
        assert_eq!(first.num_items(), 10);
        assert_eq!(rest.num_items(), 5);
        assert_eq!(
            record_times(&first),
            (1..=10).map(|i| i.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            record_times(&rest),
            (11..=15).map(|i| i.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn split_items_takes_tail_when_mid_is_smaller_half() {
        // mid < len/2 (5 of 15): `.0` still holds `mid` items, but it is the
        // *trailing* chunk, so the two pieces are swapped relative to the input.
        let records: Vec<_> = (1..=15).map(record_with_time).collect();
        let pl = logs_payload(vec![resource_logs(vec![], records)]);

        let (first, rest) = pl.split_items(5).unwrap();
        let first = first.expect("first half non-empty");
        let rest = rest.expect("remainder non-empty");
        assert_eq!(first.num_items(), 5, "`.0` holds the requested item count");
        assert_eq!(rest.num_items(), 10);
        // The mid-sized piece is the tail rows; the remainder is the head rows.
        assert_eq!(
            record_times(&first),
            (11..=15).map(|i| i.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            record_times(&rest),
            (1..=10).map(|i| i.to_string()).collect::<Vec<_>>()
        );
    }

    /// Sharding masks the root table, so a metric row holding many data points
    /// must still count as one row — masking by `num_items()` would overrun the
    /// root batch.
    #[test]
    fn shard_on_metrics() {
        let dp_count = 10;
        let pl = gauge_metrics(dp_count);
        assert_eq!(attr::get_record(&pl).unwrap().num_rows(), 1); // 1 metric row
        assert_eq!(pl.num_items(), dp_count); // 10 data points

        let mut batch = PayloadBatch::new();
        batch.push(pl);

        let groups = batch.shard(AttrLocation::Resource, "k").unwrap();
        assert_eq!(groups.len(), 1); // 1 metric row
        assert_eq!(
            groups[0].1.iter().map(|p| p.num_items()).sum::<usize>(),
            dp_count
        ); // 10 data points
    }

    /// A metrics payload with one gauge row carrying `dp_count` data points and
    /// resource attribute `k = v`.
    fn gauge_metrics(dp_count: usize) -> Payload {
        let dp = |v: f64| NumberDataPoint {
            attributes: vec![],
            start_time_unix_nano: 0,
            time_unix_nano: 1785316096797834403,
            exemplars: vec![],
            flags: 0,
            value: Some(number_data_point::Value::AsDouble(v)),
        };
        metrics_payload(MetricsData {
            resource_metrics: vec![ProtoResourceMetrics {
                resource: Some(Resource {
                    attributes: vec![kv("k", Value::StringValue("v".into()))],
                    ..Default::default()
                }),
                scope_metrics: vec![ProtoScopeMetrics {
                    scope: None,
                    metrics: vec![Metric {
                        name: "m".to_string(),
                        description: String::new(),
                        unit: String::new(),
                        metadata: vec![],
                        data: Some(metric::Data::Gauge(Gauge {
                            data_points: (0..dp_count).map(|i| dp(i as f64)).collect(),
                        })),
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        })
    }

    /// For metrics the two counts diverge in both directions: many data points on
    /// one row, or a row with none at all.
    #[test]
    fn num_rows_differs_from_num_items_for_metrics() {
        let many = gauge_metrics(10);
        assert_eq!(many.num_rows(), 1);
        assert_eq!(many.num_items(), 10);

        let none = gauge_metrics(0);
        assert_eq!(none.num_rows(), 1, "the metric row still exists");
        assert_eq!(none.num_items(), 0);
        assert!(
            !none.is_empty(),
            "is_empty() asks about records, so a row with no data points is not empty"
        );

        // Logs keep the two in lockstep.
        let logs = logs_payload(vec![resource_logs(vec![], vec![record(vec![]); 3])]);
        assert_eq!(logs.num_rows(), 3);
        assert_eq!(logs.num_items(), 3);
    }

    /// The batch-level counts mirror the payload-level ones: `num_items` totals
    /// data points, `num_rows` totals records, and `is_empty` asks whether there
    /// are any payloads at all — which is the question flush gates need, since a
    /// batch of data-point-less metric rows counts zero items but is not empty.
    #[test]
    fn payload_batch_counts_items_rows_and_emptiness_separately() {
        let mut batch = PayloadBatch::new();
        assert!(batch.is_empty());
        assert_eq!(batch.num_payloads(), 0);
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.num_items(), 0);

        batch.push(gauge_metrics(6));
        batch.push(logs_payload(vec![resource_logs(
            vec![],
            vec![record(vec![]); 2],
        )]));

        assert!(!batch.is_empty());
        assert_eq!(batch.num_payloads(), 2);
        assert_eq!(batch.num_rows(), 3, "1 metric row + 2 log records");
        assert_eq!(batch.num_items(), 8, "6 data points + 2 log records");

        // A batch holding only data-point-less metric rows: zero items, but it
        // still has records and must not read as empty.
        let mut sparse = PayloadBatch::new();
        sparse.push(gauge_metrics(0));
        assert_eq!(sparse.num_items(), 0);
        assert_eq!(sparse.num_rows(), 1);
        assert!(!sparse.is_empty());
    }

    /// A metric row with no data points has `num_items() == 0`; sharding must key
    /// off the root row count so the record is not silently discarded.
    #[test]
    fn shard_keeps_metric_rows_without_data_points() {
        let mut batch = PayloadBatch::new();
        batch.push(gauge_metrics(0));

        let groups = batch.shard(AttrLocation::Resource, "k").unwrap();
        assert_eq!(groups.len(), 1, "row must not be dropped");
        assert_eq!(groups[0].0.as_ref().map(|z| z.as_ref()), Some("v"));
        let rows: usize = groups[0].1.iter().map(|p| p.num_rows()).sum();
        assert_eq!(rows, 1);
    }

    #[test]
    fn split_items_zero_keeps_everything_in_rest() {
        let pl = logs_payload(vec![resource_logs(vec![], vec![record(vec![])])]);
        let (first, rest) = pl.split_items(0).unwrap();
        assert!(first.is_none());
        assert_eq!(rest.unwrap().num_items(), 1);
    }

    #[test]
    fn split_items_past_end_keeps_everything_in_first() {
        let pl = logs_payload(vec![resource_logs(vec![], vec![record(vec![])])]);
        let (first, rest) = pl.split_items(10).unwrap();
        assert_eq!(first.unwrap().num_items(), 1);
        assert!(rest.is_none());
    }
}
