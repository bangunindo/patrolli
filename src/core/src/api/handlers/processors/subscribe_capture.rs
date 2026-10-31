// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use axum::{
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use http::StatusCode;
use serde::Serialize;
use tokio::select;
use tracing::{Instrument, debug, info_span, warn};
use uuid::Uuid;

use crate::{
    api::{response::ResponseError, state::AppState},
    cluster::inter::v1::CaptureRequest,
};

#[derive(Debug, Serialize)]
pub struct CaptureResponse<'a> {
    pub index: u64,
    pub message_in: &'a serde_json::value::RawValue,
    pub message_out: Option<&'a serde_json::value::RawValue>,
}

pub async fn ws_subscribe_capture(
    State(state): State<Arc<AppState>>,
    Path(processor_id): Path<Uuid>,
    ws: WebSocketUpgrade,
) -> Response {
    // `on_upgrade` runs the socket task outside the request span; parent the session span on it
    // explicitly so `request_id`/`user` carry over.
    let span = info_span!("capture_session", processor_id = %processor_id);
    ws.on_upgrade(move |socket| handle_socket(state, processor_id, socket).instrument(span))
}

// spawned on each ws connection
async fn handle_socket(state: Arc<AppState>, processor_id: Uuid, mut socket: WebSocket) {
    let request = CaptureRequest {
        max_count: 10,
        processor_id: processor_id.as_bytes().to_vec().into(),
    };

    let capture_stream = match state.inter_comm.capture(request).await {
        Ok(stream) => Some(stream),
        Err(e) => {
            warn!(error = %e, "failed to start capture");
            let err_resp = ResponseError::from_status(StatusCode::INTERNAL_SERVER_ERROR)
                .with_title("Failed to start capture")
                .with_detail(format!("Error: {}", e))
                .to_json_string();
            socket.send(Message::Text(err_resp.into())).await.ok();
            socket.send(Message::Close(None)).await.ok();
            return;
        }
    };

    let Some(capture_stream) = capture_stream else {
        warn!("failed to start capture, no capture stream");
        let err_resp = ResponseError::from_status(StatusCode::INTERNAL_SERVER_ERROR)
            .with_title("Failed to start capture")
            .with_detail("Capture stream is not found")
            .to_json_string();
        socket.send(Message::Text(err_resp.into())).await.ok();
        socket.send(Message::Close(None)).await.ok();
        return;
    };

    debug!("capture session opened");
    let mut index: u64 = 0;
    loop {
        select! {
            captured = capture_stream.receiver.recv_async() => {
                match captured {
                    Ok(captured) => {
                        index += 1;

                        let in_msg = match serde_json::from_slice(&captured.message_in) {
                            Ok(json) => json,
                            Err(e) => {
                                debug!(error = %e, "failed to parse capture message_in as json");
                                continue;
                            }
                        };
                        let out_msg = match &captured.message_out {
                            Some(msg) => match serde_json::from_slice(msg) {
                                Ok(json) => Some(json),
                                Err(e) => {
                                    debug!(error = %e, "failed to parse capture message_out as json");
                                    None
                                }
                            },
                            None => None,
                        };

                        let response = CaptureResponse {
                            index,
                            message_in: in_msg,
                            message_out: out_msg,
                        };

                        let response_json = match serde_json::to_string(&response) {
                            Ok(json) => json,
                            Err(e) => {
                                debug!(error = %e, "failed to serialize capture response");
                                continue;
                            }
                        };

                        if socket.send(Message::Text(response_json.into())).await.is_err() {
                            debug!(reason = "send_failed", "capture session closed");
                            return;
                        }
                    }
                    Err(_) => {
                        // capture stream ended (sender dropped)
                        break;
                    }
                }
            }
            ws_msg = socket.recv() => {
                match ws_msg {
                    Some(Ok(Message::Close(_))) | None => {
                        debug!(reason = "client_disconnected", "capture session closed");
                        return;
                    }
                    Some(Ok(_)) => {
                        // ignore other client messages
                        continue;
                    }
                    Some(Err(e)) => {
                        debug!(error = %e, reason = "websocket_error", "capture session closed");
                        return;
                    }
                }
            }
        }
    }

    // capture stream ended, send a close message to the client
    socket.send(Message::Close(None)).await.ok();
    debug!(reason = "stream_ended", "capture session closed");
}
