// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Compiles the internal cluster protobufs into Rust via tonic-prost-build.
//!
//! Output lands in `$OUT_DIR/<proto package>.rs` and is pulled into the crate
//! by `tonic::include_proto!` (see `src/cluster/inter/mod.rs`). Requires
//! `protoc` (the `protobuf-compiler` package) on PATH.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Inter-node cluster RPC: gRPC + protobuf, patrolli-to-patrolli only.
    const INTER_CLUSTER_PROTO: &str = "src/cluster/inter/inter_cluster.proto";
    const INTER_CLUSTER_INCLUDE: &str = "src/cluster/inter";

    println!("cargo:rerun-if-changed={INTER_CLUSTER_PROTO}");
    // `embed_migrations!` (src/db/mod.rs) reads this directory when it expands,
    // which cargo does not track: without this, a new migration is left out of
    // the binary until something else rebuilds the crate.
    println!("cargo:rerun-if-changed=migrations");

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        // Generate `bytes::Bytes` instead of `Vec<u8>` for all `bytes` fields.
        .bytes(".")
        // Reuse opentelemetry_proto's generated OTLP types instead of generating a
        // second incompatible copy; they were built from the same proto files.
        .compile_protos(&[INTER_CLUSTER_PROTO], &[INTER_CLUSTER_INCLUDE])?;

    Ok(())
}
