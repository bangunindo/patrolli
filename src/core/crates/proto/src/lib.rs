// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Generated gRPC bindings for the internal inter-node cluster service.
//!
//! The protocol is defined in `inter_cluster.proto` next to this file and
//! compiled by `build.rs` (tonic-prost-build) into `$OUT_DIR`. We include the
//! generated module here under a versioned path so the rest of the workspace refers
//! to it as `patrolli_proto::v1::...`.
//!
//! This is a private patrolli-to-patrolli protocol, not an OpenTelemetry endpoint.

#[allow(clippy::all)]
pub mod v1 {
    tonic::include_proto!("patrolli.cluster.inter.v1");
}
