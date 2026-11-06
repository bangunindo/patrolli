// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Compiles the internal cluster protobufs into Rust via tonic-prost-build.
//!
//! Output lands in `$OUT_DIR/<proto package>.rs` and is pulled into the crate
//! by `tonic::include_proto!` (see `src/lib.rs`). Requires
//! `protoc` (the `protobuf-compiler` package) on PATH.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Inter-node cluster RPC: gRPC + protobuf, patrolli-to-patrolli only.
    const INTER_CLUSTER_PROTO: &str = "src/inter_cluster.proto";
    const INTER_CLUSTER_INCLUDE: &str = "src";

    println!("cargo:rerun-if-changed={INTER_CLUSTER_PROTO}");

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        // Generate `bytes::Bytes` instead of `Vec<u8>` for all `bytes` fields.
        .bytes(".")
        .compile_protos(&[INTER_CLUSTER_PROTO], &[INTER_CLUSTER_INCLUDE])?;

    Ok(())
}
