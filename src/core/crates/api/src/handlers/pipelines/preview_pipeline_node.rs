// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::error::ApiResultExt;
use crate::handlers::pipelines::graph::{
    GraphNodeContent, GraphNodeKind, GraphProcessorGroupContentInput, GraphProcessorInput,
};
use crate::handlers::pipelines::runtime::load_node_content;
use crate::handlers::pipelines::{find_canvas, find_pipeline, node_not_found};
use crate::jwt::AuthUser;
use crate::response::ResponseError;
use crate::state::AppState;
use anyhow::anyhow;
use axum::Extension;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::Response;
use patrolli::cache::Cache;
use patrolli::config::Format;
use patrolli::config::processor::{Config, Processor};
use patrolli::db::schema::{AdmStatus, PipelineNodeProcessorGroups, PipelineNodes};
use patrolli::payload::Payload;
use patrolli_cluster::inter::CaptureStream;
use patrolli_processor::preview::{Chain, split_records};
use patrolli_proto::v1::capture_message::Sample;
use patrolli_proto::v1::{CaptureMessage, CaptureMessageOtlp, CaptureRequest, SignalType};
use sea_query::{Expr, ExprTrait, PostgresQueryBuilder, Query as DbQuery};
use sea_query_postgres::PostgresBinder;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::select;
use tokio_postgres::IsolationLevel;
use tracing::{Instrument, Span, debug, info_span, trace};
use uuid::Uuid;

const MAX_BUFFERED_RECORDS: usize = 100;
const MAX_MESSAGE_SIZE: usize = 1 << 20;
const TICK: Duration = Duration::from_secs(5);
/// Browsers reply to our pings on their own, so a live client sends something every `TICK`.
const IDLE_TIMEOUT: Duration = Duration::from_secs(15);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    /// Swap the chain being previewed. The server replies with `reset`, then replays every
    /// buffered record through the new chain. If the draft's processors fail to parse or
    /// validate, the server replies with `draft_rejected` and keeps the old chain.
    Draft {
        id: u64,
        processors: serde_json::Value,
    },
    /// Stop taking new samples. The buffer is kept, so a new draft still runs on it.
    Pause,
    /// Start taking new samples again after a pause.
    Resume,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage<'a> {
    /// Sent when a draft is accepted. The client should throw away the records it has, because
    /// every record after this comes from the new draft.
    Reset { draft_id: u64 },
    /// One record of a sample, before and after the chain. `error` is set in two cases: the chain
    /// failed (`after` is empty), or a lookup rule was skipped (`after` still has the output).
    Record {
        /// Id of the draft that produced `after`, or null for the saved chain.
        draft_id: Option<u64>,
        sample: u64,
        index: usize,
        before: &'a RawValue,
        /// 1 input record can transform into:
        ///     - 0 records (if it's filtered out)
        ///     - 1 record (normal transformation)
        ///     - many records (split into multiple records)
        after: Vec<Box<RawValue>>,
        error: Option<String>,
    },
    /// The server dropped this sample from its buffer to stay under `MAX_BUFFERED_RECORDS`,
    /// so later drafts will not replay it.
    Evict { sample: u64 },
    /// The draft's processors failed to parse or validate. The server keeps the old chain.
    DraftRejected { draft_id: u64, detail: String },
    /// Something went wrong and the session is over. The server closes the socket right after.
    Error { detail: String },
}

struct BufferedRecord {
    payload: Payload,
    before: Box<RawValue>,
}

struct BufferedSample {
    seq: u64,
    records: Vec<BufferedRecord>,
}

pub async fn ws_preview_pipeline_node(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthUser>,
    Path((pipeline_id, canvas_id, node_id)): Path<(Uuid, Uuid, Uuid)>,
    ws: WebSocketUpgrade,
) -> Result<Response, ResponseError> {
    let mut client = state.db_pool.get().await.ctx("get database connection")?;
    let tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .ctx("start transaction")?;

    // check pipeline is visible and owns the canvas and the node
    find_pipeline(&tx, pipeline_id, auth_user.user_id, false).await?;
    find_canvas(&tx, pipeline_id, canvas_id).await?;
    let (sql, values) = DbQuery::select()
        .column(PipelineNodes::Kind)
        .from(PipelineNodes::Table)
        .and_where(Expr::col(PipelineNodes::NodeId).eq(node_id))
        .and_where(Expr::col(PipelineNodes::CanvasId).eq(canvas_id))
        .build_postgres(PostgresQueryBuilder);
    let row = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query node")?
        .ok_or_else(|| node_not_found(node_id))?;
    let kind: String = row.get(0);
    let kind: GraphNodeKind = kind.parse().ctx("parse node kind")?;

    if !matches!(kind, GraphNodeKind::ProcessorGroup) {
        return Err(ResponseError::from_status(StatusCode::BAD_REQUEST)
            .with_title("Only processor group nodes have a preview"));
    }

    // get the group's input processor to tap
    let (sql, values) = DbQuery::select()
        .column(PipelineNodeProcessorGroups::InputProcessorId)
        .from(PipelineNodeProcessorGroups::Table)
        .and_where(Expr::col(PipelineNodeProcessorGroups::NodeId).eq(node_id))
        .build_postgres(PostgresQueryBuilder);
    let tap: Uuid = tx
        .query_opt(sql.as_str(), &values.as_params())
        .await
        .ctx("query tap processor")?
        .ok_or_else(|| node_not_found(node_id))?
        .get(0);

    // load the saved chain, the starting draft
    let GraphNodeContent::ProcessorGroup(content) =
        load_node_content(&tx, node_id, kind, None, 0, 0)
            .await
            .ctx("load node content")?
    else {
        return Err(
            ResponseError::from_status(StatusCode::INTERNAL_SERVER_ERROR)
                .with_title("Node content does not match its kind"),
        );
    };
    let saved = content
        .processors
        .into_iter()
        .filter(|p| !matches!(p.adm_status, AdmStatus::Disabled))
        .map(|mut p| {
            p.config.validate()?;
            Ok(to_processor(p.processor_id, p.name, p.config))
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .ctx("validate saved processors")?;
    tx.commit().await.ctx("commit transaction")?;

    // `on_upgrade` runs the socket task outside the request span, so attach the session span
    // to it by hand to keep `request_id` and `user` in the logs.
    let span = info_span!("preview_session", pipeline_node_id = %node_id);
    Ok(ws
        .max_message_size(MAX_MESSAGE_SIZE)
        .max_frame_size(MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| run_session(state, tap, saved, socket).instrument(span)))
}

fn to_processor(id: Uuid, name: String, config: Config) -> Arc<Processor> {
    Arc::new(Processor {
        id,
        name,
        config,
        inputs: vec![],
    })
}

fn parse_draft(processors: serde_json::Value) -> anyhow::Result<Vec<Arc<Processor>>> {
    let processors: Vec<GraphProcessorInput> = serde_json::from_value(processors)?;
    Ok(GraphProcessorGroupContentInput { processors }
        .validate()?
        .into_iter()
        .filter(|p| !matches!(p.adm_status, AdmStatus::Disabled))
        .map(|p| to_processor(p.processor_id, p.name, p.config))
        .collect())
}

async fn send_frame(socket: &mut WebSocket, message: Message) -> Result<(), axum::Error> {
    tokio::time::timeout(SEND_TIMEOUT, socket.send(message))
        .await
        .map_err(axum::Error::new)?
}

async fn send(socket: &mut WebSocket, message: &ServerMessage<'_>) -> Result<(), axum::Error> {
    let json = serde_json::to_string(message).map_err(axum::Error::new)?;
    send_frame(socket, Message::Text(json.into())).await
}

async fn fail(socket: &mut WebSocket, detail: String) -> Result<(), axum::Error> {
    send(socket, &ServerMessage::Error { detail }).await?;
    send_frame(socket, Message::Close(None)).await
}

async fn subscribe(state: &AppState, tap: Uuid) -> anyhow::Result<CaptureStream> {
    state
        .inter_comm
        .capture(CaptureRequest {
            max_count: 10,
            processor_id: tap.as_bytes().to_vec().into(),
        })
        .await
}

async fn run_session(
    state: Arc<AppState>,
    tap: Uuid,
    saved: Vec<Arc<Processor>>,
    mut socket: WebSocket,
) {
    debug!("preview session opened");
    match drive(&state, tap, saved, &mut socket).await {
        Ok(reason) => debug!(reason, "preview session closed"),
        Err(e) => debug!(error = %e, reason = "send_failed", "preview session closed"),
    }
}

async fn drive(
    state: &Arc<AppState>,
    tap: Uuid,
    saved: Vec<Arc<Processor>>,
    socket: &mut WebSocket,
) -> Result<&'static str, axum::Error> {
    let mut preview = Preview {
        cache: state.cache.clone(),
        processors: saved,
        draft: None,
        buffer: VecDeque::new(),
        buffered_records: 0,
        next_seq: 0,
    };

    let mut capture = match subscribe(state, tap).await {
        Ok(stream) => Some(stream),
        Err(e) => {
            debug!(error = %format!("{e:#}"), "failed to start capture");
            fail(socket, format!("failed to start capture: {e:#}")).await?;
            return Ok("capture_failed");
        }
    };

    let mut tick = tokio::time::interval(TICK);
    let mut last_seen = Instant::now();
    loop {
        select! {
            captured = async {
                capture.as_ref().expect("guarded by the select precondition").receiver.recv_async().await
            }, if capture.is_some() => {
                match captured {
                    Ok(CaptureMessage { sample: Some(Sample::Otlp(otlp)) }) => {
                        let Some((returned, result)) = run_blocking(move || {
                            let result = preview.add_sample(&otlp);
                            (preview, result)
                        })
                        .await else {
                            return Ok("run_failed");
                        };
                        preview = returned;
                        match result {
                            Ok(messages) => send_all(socket, messages).await?,
                            Err(e) => debug!(error = %format!("{e:#}"), "failed to read capture sample"),
                        }
                    }
                    Ok(_) => debug!("capture sample in an unknown format"),
                    Err(_) => {
                        fail(socket, "capture stream ended".into()).await?;
                        return Ok("stream_ended");
                    }
                }
            }
            _ = tick.tick() => {
                if last_seen.elapsed() > IDLE_TIMEOUT {
                    return Ok("client_timeout");
                }
                if state.cache.processor.read().get_step(tap).is_none() {
                    fail(socket, "node was removed".into()).await?;
                    return Ok("node_removed");
                }
                send_frame(socket, Message::Ping(Default::default())).await?;
            }
            message = socket.recv() => {
                if let Some(Ok(_)) = message {
                    last_seen = Instant::now();
                }
                match message {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMessage>(&text) {
                        Ok(ClientMessage::Draft { id, processors }) => match parse_draft(processors) {
                            Ok(processors) => {
                                send(socket, &ServerMessage::Reset { draft_id: id }).await?;
                                let Some((returned, messages)) = run_blocking(move || {
                                    let messages = preview.set_draft(id, processors);
                                    (preview, messages)
                                })
                                .await else {
                                    return Ok("run_failed");
                                };
                                preview = returned;
                                send_all(socket, messages).await?;
                            }
                            Err(e) => {
                                let detail = format!("{e:#}");
                                send(socket, &ServerMessage::DraftRejected { draft_id: id, detail }).await?;
                            }
                        },
                        Ok(ClientMessage::Pause) => capture = None,
                        Ok(ClientMessage::Resume) => {
                            if capture.is_none() {
                                match subscribe(state, tap).await {
                                    Ok(stream) => {
                                        capture = Some(stream);
                                        // nothing is read while subscribe waits on every peer, so reset the idle clock
                                        last_seen = Instant::now();
                                    }
                                    Err(e) => {
                                        debug!(error = %format!("{e:#}"), "failed to resume capture");
                                        fail(socket, format!("failed to start capture: {e:#}")).await?;
                                        return Ok("capture_failed");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            fail(socket, format!("invalid message: {e}")).await?;
                            return Ok("bad_message");
                        }
                    },
                    Some(Ok(Message::Binary(_))) => {
                        fail(socket, "invalid message: expected a text frame".into()).await?;
                        return Ok("bad_message");
                    }
                    Some(Ok(Message::Close(_))) | None => return Ok("client_disconnected"),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        debug!(error = %e, "websocket error");
                        return Ok("websocket_error");
                    }
                }
            }
        }
    }
}

async fn run_blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let span = Span::current();
    tokio::task::spawn_blocking(move || span.in_scope(f))
        .await
        .ok()
}

async fn send_all(socket: &mut WebSocket, messages: Vec<String>) -> Result<(), axum::Error> {
    for json in messages {
        send_frame(socket, Message::Text(json.into())).await?;
    }
    Ok(())
}

fn to_otlp_json(pl: &Payload) -> anyhow::Result<Box<RawValue>> {
    let json = pl
        .serialize_as(Format::JsonOtlpBatch)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("empty json"))?;
    Ok(RawValue::from_string(String::from_utf8(Vec::from(json))?)?)
}

fn read_sample(otlp: &CaptureMessageOtlp, seq: u64) -> anyhow::Result<BufferedSample> {
    let signal = SignalType::try_from(otlp.signal)?;
    let payload = Payload::from_proto_otlp(signal, &otlp.message_in)?;
    let records = split_records(payload)?
        .into_iter()
        .map(|payload| {
            let before = to_otlp_json(&payload)?;
            Ok(BufferedRecord { payload, before })
        })
        .collect::<anyhow::Result<_>>()?;
    Ok(BufferedSample { seq, records })
}

struct Preview {
    cache: Cache,
    processors: Vec<Arc<Processor>>,
    draft: Option<u64>,
    buffer: VecDeque<BufferedSample>,
    buffered_records: usize,
    next_seq: u64,
}

impl Preview {
    fn add_sample(&mut self, otlp: &CaptureMessageOtlp) -> anyhow::Result<Vec<String>> {
        let sample = read_sample(otlp, self.next_seq)?;
        if sample.records.is_empty() {
            return Ok(Vec::new());
        }
        self.next_seq += 1;
        self.buffered_records += sample.records.len();
        self.buffer.push_back(sample);
        let mut messages = Vec::new();
        while self.buffered_records > MAX_BUFFERED_RECORDS && self.buffer.len() > 1 {
            let old = self
                .buffer
                .pop_front()
                .expect("buffer holds more than one sample");
            self.buffered_records -= old.records.len();
            match serde_json::to_string(&ServerMessage::Evict { sample: old.seq }) {
                Ok(json) => messages.push(json),
                Err(e) => debug!(error = %e, "failed to serialize evict message"),
            }
        }
        let newest = self.buffer.back().expect("sample was just pushed");
        messages.extend(self.run_sample(&Chain::new(self.processors.clone()), newest));
        Ok(messages)
    }

    fn set_draft(&mut self, id: u64, processors: Vec<Arc<Processor>>) -> Vec<String> {
        let chain = Chain::new(processors.clone());
        self.processors = processors;
        self.draft = Some(id);
        self.buffer
            .iter()
            .flat_map(|sample| self.run_sample(&chain, sample))
            .collect()
    }

    fn run_sample(&self, chain: &Chain, sample: &BufferedSample) -> Vec<String> {
        sample
            .records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| {
                let result =
                    chain
                        .run(record.payload.clone(), &self.cache)
                        .and_then(|(pl, skipped)| {
                            let after = split_records(pl)?
                                .iter()
                                .map(to_otlp_json)
                                .collect::<anyhow::Result<_>>()?;
                            Ok((after, skipped))
                        });
                let (after, error) = match result {
                    Ok((after, skipped)) => (after, skipped.map(|e| format!("{e:#}"))),
                    Err(e) => (Vec::new(), Some(format!("{e:#}"))),
                };
                serde_json::to_string(&ServerMessage::Record {
                    draft_id: self.draft,
                    sample: sample.seq,
                    index,
                    before: &record.before,
                    after,
                    error,
                })
                .inspect_err(|e| trace!(error = %e, "failed to serialize record message"))
                .ok()
            })
            .collect()
    }
}
