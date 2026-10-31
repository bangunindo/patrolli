// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::cluster::inter::v1::inter_cluster_service_client::InterClusterServiceClient;
use crate::cluster::inter::v1::{CaptureMessage, CaptureRequest};
use anyhow::Result;
use flume::Sender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::{Request, transport::Channel};
use tracing::Instrument;

pub(super) struct CaptureClient {
    transport: Channel,
    bearer: super::comm::Bearer,
}

impl CaptureClient {
    pub(super) fn new(transport: Channel, bearer: super::comm::Bearer) -> Self {
        Self { transport, bearer }
    }

    pub(super) async fn capture(
        &self,
        req: CaptureRequest,
        sender: Sender<CaptureMessage>,
        token: CancellationToken,
    ) -> Result<JoinHandle<()>> {
        let mut client = InterClusterServiceClient::new(self.transport.clone());
        let mut request = Request::new(req);
        self.bearer.apply(&mut request);
        let mut stream = client.processor_capture(request).await?.into_inner();
        Ok(tokio::spawn(
            async move {
                loop {
                    tokio::select! {
                        _ = token.cancelled() => break,
                        msg = stream.message() => match msg {
                            Ok(Some(m)) => if sender.send_async(m).await.is_err() { break },
                            Ok(None) => break,
                            Err(e) => { tracing::warn!(error = %e, "capture stream error"); break }
                        }
                    }
                }
            }
            .in_current_span(),
        ))
    }
}
