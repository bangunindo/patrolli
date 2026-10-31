// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Generated gRPC bindings for the internal inter-node cluster service.
//!
//! The protocol is defined in `inter_cluster.proto` next to this file and
//! compiled by `build.rs` (tonic-prost-build) into `$OUT_DIR`. We include the
//! generated module here under a versioned path so the rest of the crate refers
//! to it as `crate::cluster::inter::v1::...`.
//!
//! This is a private patrolli-to-patrolli protocol — not an OpenTelemetry endpoint.

mod capture;
mod comm;
mod forwarder;
mod item_stats;

pub use item_stats::to_proto as item_stats_to_proto;

pub use comm::{
    Bearer, CaptureStream, FORWARDER_CHANNEL_NAME, InterComm, InterCommManager, InterCommServer,
    InterCommService, MAX_GRPC_MESSAGE_SIZE,
};

#[allow(clippy::all)]
pub mod v1 {
    tonic::include_proto!("patrolli.cluster.inter.v1");
}
