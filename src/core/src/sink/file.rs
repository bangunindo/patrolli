// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::export::{Encoded, Export, ExportSink, Exporter};
use crate::config::Format;
use crate::{chan, config, payload};
use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::error;

/// Token bucket: `per_second` tokens a second, earned continuously, holding
/// at most 10 ms worth (at least one) so a quiet spell does not turn into a
/// burst. Counting every token earned since `start`, rather than per refill,
/// loses no fraction of one, so any rate from 1 a second up holds exactly.
struct Throttle {
    per_second: u128,
    capacity: u128,
    available: u128,
    start: Instant,
    /// Tokens earned from `start` up to the last refill.
    earned: u128,
}

const NANOS_PER_SEC: u128 = 1_000_000_000;

impl Throttle {
    /// `per_second` – max messages per second, above 0.
    fn new(per_second: usize) -> Self {
        let per_second = per_second as u128;
        let capacity = (per_second / 100).max(1);
        Self {
            per_second,
            capacity,
            available: capacity,
            start: Instant::now(),
            earned: 0,
        }
    }

    /// Try to acquire one token. Returns `true` if allowed.
    fn try_acquire(&mut self) -> bool {
        self.refill();
        if self.available > 0 {
            self.available -= 1;
            true
        } else {
            false
        }
    }

    /// Sleep until the next token is earned.
    async fn wait_for_tokens(&mut self) {
        // The first instant `t` with `(t - start) * per_second >= (earned + 1) s`.
        let nanos = ((self.earned + 1) * NANOS_PER_SEC).div_ceil(self.per_second);
        tokio::time::sleep_until(self.start + Duration::from_nanos(nanos as u64)).await;
        self.refill();
    }

    fn refill(&mut self) {
        let earned = self.start.elapsed().as_nanos() * self.per_second / NANOS_PER_SEC;
        self.available = (self.available + earned - self.earned).min(self.capacity);
        self.earned = earned;
    }
}

/// One serialized blob and the items it carries (see [`super::blob_items`]).
#[derive(Clone)]
pub struct Blob {
    bytes: Bytes,
    items: u64,
}

pub struct Blobs(Vec<Blob>);

pub struct File {
    format: Format,
    output: Output,
    /// Shared by the sink's workers, so `throttle` is a rate for the sink.
    /// `None` when unlimited.
    throttle: Option<Mutex<Throttle>>,
}

/// Where records go. Written to on a blocking thread, see [`Output::write`].
#[derive(Clone)]
enum Output {
    /// Appended to. The lock keeps each run of records whole between the
    /// sink's workers.
    File(Arc<parking_lot::Mutex<std::fs::File>>),
    Stdout,
    Stderr,
    Null,
}

impl Output {
    /// Writes `records` in order, each one whole: a record that fails partway
    /// through a regular file is cut off again, so the file never holds half
    /// of one. Stops at a record boundary once `stop` is cancelled. How many
    /// it wrote, and what stopped it short.
    fn write(
        &self,
        records: &[Bytes],
        format: Format,
        stop: &CancellationToken,
    ) -> (usize, Option<io::Error>) {
        let mut buf = Vec::new();
        let mut write = |out: &mut dyn Write, record: &Bytes| {
            buf.clear();
            match format {
                Format::JsonOtlpDenormalized
                | Format::JsonNestedBatch
                | Format::JsonOtlpBatch
                | Format::JsonNestedDenormalized => {
                    buf.extend_from_slice(record);
                    buf.push(b'\n');
                }
                Format::ProtoOtlp => {
                    buf.extend_from_slice(&(record.len() as u32).to_be_bytes());
                    buf.extend_from_slice(record);
                }
            }
            out.write_all(&buf)
        };
        let mut written = 0;
        let result = (|| match self {
            Output::File(file) => {
                let mut file = file.lock();
                for record in records {
                    if stop.is_cancelled() {
                        break;
                    }
                    // Appends land at the end, which only this lock moves.
                    let start = file.metadata()?.len();
                    if let Err(e) = write(&mut *file, record) {
                        _ = file.set_len(start);
                        return Err(e);
                    }
                    written += 1;
                }
                Ok(())
            }
            Output::Stdout | Output::Stderr => {
                let mut out: Box<dyn Write> = match self {
                    Output::Stdout => Box::new(io::stdout().lock()),
                    _ => Box::new(io::stderr().lock()),
                };
                for record in records {
                    if stop.is_cancelled() {
                        break;
                    }
                    write(&mut *out, record)?;
                    written += 1;
                }
                out.flush()
            }
            Output::Null => {
                written = records.len();
                Ok(())
            }
        })();
        (written, result.err())
    }
}

pub type FileSink = ExportSink<File>;

impl FileSink {
    // `Worker::new` builds from a `Prepared` exporter; this is for tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn new(
        conf: config::sink::Sink,
        channel: chan::Consumer<payload::Payload>,
    ) -> Result<Self> {
        let exporter = File::new(&conf).await?;
        Ok(Self::with_exporter(conf, channel, exporter))
    }
}

impl File {
    pub(super) async fn new(conf: &config::sink::Sink) -> Result<Self> {
        let config::sink::Config::File(file_conf) = &conf.config else {
            panic!("not file config")
        };
        let output = match file_conf.path.as_str() {
            "stdout" => Output::Stdout,
            "stderr" => Output::Stderr,
            "null" => Output::Null,
            path => {
                // Appended to: a restart carries on after what is there.
                let file = tokio::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(path)
                    .await?
                    .into_std()
                    .await;
                Output::File(Arc::new(parking_lot::Mutex::new(file)))
            }
        };
        Ok(Self {
            format: file_conf.format,
            output,
            throttle: (file_conf.throttle > 0)
                .then(|| Mutex::new(Throttle::new(file_conf.throttle))),
        })
    }

    /// How many of the `left` blobs the throttle lets out now, at least one.
    async fn allowed(&self, left: usize) -> usize {
        let Some(throttle) = &self.throttle else {
            return left;
        };
        let mut throttle = throttle.lock().await;
        while !throttle.try_acquire() {
            throttle.wait_for_tokens().await;
        }
        let mut n = 1;
        while n < left && throttle.try_acquire() {
            n += 1;
        }
        n
    }

    /// The blobs from `failed` on did not make it; those before did.
    fn failed(&self, blobs: &Blobs, failed: usize, error: io::Error) -> Export<Blobs> {
        let retry = error.kind() == io::ErrorKind::StorageFull;
        let error = anyhow::Error::new(error).context("failed to write to file");
        if failed == 0 {
            return if retry {
                Export::Retry { error, after: None }
            } else {
                Export::Reject { error }
            };
        }
        let left = Blobs(blobs.0[failed..].to_vec());
        if retry {
            Export::Partial {
                retry: Some((left, error)),
                rejected: None,
            }
        } else {
            Export::Partial {
                retry: None,
                rejected: Some((self.items(&left), error)),
            }
        }
    }
}

#[async_trait]
impl Exporter for File {
    type Encoded = Blobs;

    fn encode(&self, batch: payload::PayloadBatch) -> Encoded<Blobs> {
        let mut blobs = Vec::new();
        let mut errored = 0;
        // Every payload, even one counting no items: a metrics payload whose
        // rows carry no data points still has records to write.
        for pl in batch {
            let items = pl.num_items() as u64;
            match pl.serialize_as(self.format) {
                Ok(serialized) => {
                    let count = serialized.len();
                    blobs.extend(serialized.into_iter().enumerate().map(|(idx, bytes)| Blob {
                        bytes,
                        items: super::blob_items(items, count, idx),
                    }));
                }
                Err(e) => {
                    errored += items;
                    error!(error = %format!("{e:#}"), records = items, "failed to serialize record");
                }
            }
        }
        Encoded {
            data: (!blobs.is_empty()).then_some(Blobs(blobs)),
            filtered: 0,
            errored,
        }
    }

    fn items(&self, blobs: &Blobs) -> u64 {
        blobs.0.iter().map(|b| b.items).sum()
    }

    /// Cancel-safe at record granularity: the blocking write finishes the
    /// record it is on when this is dropped, and writes no further one. The
    /// records it did write come again with the requeued batch.
    async fn export(&self, blobs: &Blobs) -> Export<Blobs> {
        let stop = CancellationToken::new();
        let _stop_on_drop = stop.clone().drop_guard();
        let mut done = 0;
        while done < blobs.0.len() {
            let n = self.allowed(blobs.0.len() - done).await;
            let records: Vec<Bytes> = blobs.0[done..done + n]
                .iter()
                .map(|b| b.bytes.clone())
                .collect();
            let (output, format, stop) = (self.output.clone(), self.format, stop.clone());
            let (written, error) =
                tokio::task::spawn_blocking(move || output.write(&records, format, &stop))
                    .await
                    .unwrap_or_else(|e| (0, Some(io::Error::other(e))));
            if let Some(e) = error {
                return self.failed(blobs, done + written, e);
            }
            done += n;
        }
        Export::Delivered
    }
}

#[cfg(test)]
mod tests {
    use super::super::Sink;
    use super::*;
    use otel_arrow_dfe_pdata::encode::encode_logs_otap_batch;
    use otel_arrow_dfe_pdata::proto::opentelemetry::logs::v1::{
        LogRecord, LogsData, ResourceLogs, ScopeLogs,
    };
    use tokio_util::sync::CancellationToken;

    fn logs(records: usize) -> payload::Payload {
        let logs = LogsData {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: vec![LogRecord::default(); records],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        payload::Payload::new(encode_logs_otap_batch(&logs).unwrap())
    }

    async fn run(path: &str, max_elapsed_ms: u64) -> super::super::Metrics {
        let mut conf: config::sink::Sink = serde_yaml_ng::from_str(&format!(
            r#"
id: 00000000-0000-0000-0000-000000000004
name: file
worker_count: 1
batch_size: 1000
batch_timeout: 3600000
retry:
  initial_interval: 10
  max_interval: 20
  max_elapsed: {max_elapsed_ms}
type: file
config:
  path: {path}
  format: json_otlp_denormalized
"#
        ))
        .unwrap();
        conf.validate().unwrap();
        let (tx, rx) = flume::bounded(4);
        let consumer = chan::Consumer::from_flume(&tx, rx);
        tx.send(logs(3)).unwrap();
        drop(tx);
        let sink = FileSink::new(conf, consumer).await.unwrap();
        sink.run(CancellationToken::new()).await;
        sink.metrics()
    }

    #[tokio::test]
    async fn every_record_is_written_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.log");
        let m = run(path.to_str().unwrap(), 0).await;
        assert_eq!(m.total_records_sent, 3);
        let written = std::fs::read_to_string(path).unwrap();
        assert_eq!(written.lines().count(), 3, "one line per record");
    }

    /// A restart carries on after what the file holds.
    #[tokio::test]
    async fn the_file_is_appended_to() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.log");
        std::fs::write(&path, "kept\n").unwrap();
        run(path.to_str().unwrap(), 0).await;
        let written = std::fs::read_to_string(path).unwrap();
        assert!(written.starts_with("kept\n"), "{written:?}");
        assert_eq!(written.lines().count(), 4);
    }

    /// A worker cancelled mid-export leaves whole records only: the write in
    /// flight finishes its record and starts no other.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancelled_export_leaves_no_half_record() {
        const RECORDS: usize = 1000;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let mut conf: config::sink::Sink = serde_yaml_ng::from_str(&format!(
            "id: 00000000-0000-0000-0000-000000000005\nname: file\ntype: file\n\
             config:\n  path: {}\n  format: proto_otlp\n",
            path.display()
        ))
        .unwrap();
        conf.validate().unwrap();
        let file = File::new(&conf).await.unwrap();
        let record = Bytes::from(vec![7u8; 64 * 1024]);
        let blobs = Blobs(vec![
            Blob {
                bytes: record.clone(),
                items: 1,
            };
            RECORDS
        ]);
        tokio::select! {
            _ = file.export(&blobs) => panic!("the export finished before it was cancelled"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => {}
        }
        // The abandoned write stops at its next record boundary.
        let mut len = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let now = std::fs::metadata(&path).unwrap().len();
            if now == len {
                break;
            }
            len = now;
        }
        let written = std::fs::read(&path).unwrap();
        let mut rest = written.as_slice();
        let mut frames = 0;
        while !rest.is_empty() {
            let (head, body) = rest.split_at(4);
            let n = u32::from_be_bytes(head.try_into().unwrap()) as usize;
            assert_eq!(&body[..n], &record[..], "frame {frames} is torn");
            rest = &body[n..];
            frames += 1;
        }
        assert!(frames < RECORDS, "the write went on after the cancel");
    }

    /// `/dev/full` fails every write with `ENOSPC`, which is retried rather
    /// than dropped, until `max_elapsed` runs out.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_full_disk_is_retried() {
        let m = run("/dev/full", 200).await;
        assert!(m.total_records_retried > 0);
        assert_eq!(m.total_records_errored, 3);
        assert_eq!(m.total_records_sent, 0);
    }

    /// How long a `per_second` throttle, taken from as fast as it allows,
    /// takes to let out its initial burst and then `n` more.
    async fn time_for(per_second: usize, n: usize) -> Duration {
        let mut throttle = Throttle::new(per_second);
        let start = Instant::now();
        for _ in 0..throttle.capacity as usize + n {
            while !throttle.try_acquire() {
                throttle.wait_for_tokens().await;
            }
        }
        start.elapsed()
    }

    /// Below 100 a second, one 10 ms tick's share rounded down to nothing,
    /// which read as unlimited.
    #[tokio::test(start_paused = true)]
    async fn a_throttle_below_100_per_second_still_throttles() {
        assert_eq!(time_for(1, 10).await, Duration::from_secs(10));
        assert_eq!(time_for(10, 100).await, Duration::from_secs(10));
        assert_eq!(time_for(99, 990).await, Duration::from_secs(10));
    }

    /// The fraction of a tick's share used to be dropped: 150 came out as 100.
    #[tokio::test(start_paused = true)]
    async fn a_rate_off_a_multiple_of_100_holds() {
        assert_eq!(time_for(150, 150).await, Duration::from_secs(1));
        assert_eq!(time_for(12_345, 12_345).await, Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_quiet_spell_banks_at_most_10_ms_of_tokens() {
        let mut throttle = Throttle::new(1000);
        tokio::time::sleep(Duration::from_secs(5)).await;
        let mut burst = 0;
        while throttle.try_acquire() {
            burst += 1;
        }
        assert_eq!(burst, 10);
    }
}
