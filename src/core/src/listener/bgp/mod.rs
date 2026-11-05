// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! BGP route collection: BMP sessions (RFC 7854, Loc-RIB per RFC 9069) and
//! passive BGP sessions (RFC 4271) that write the routes into a `bgp` lookup
//! table (see `docs/bgp-bmp.md`).

pub mod codec;
pub mod events;
pub mod listener;
#[cfg(target_os = "linux")]
pub mod md5;
pub mod paths;
pub mod peer;
pub mod routers;
pub mod routes;
pub mod session;
pub mod stats;
pub mod wire;

pub use listener::BgpListener;
pub use peer::BgpConfig;
pub use routers::Protocol;
pub use session::{BmpConfig, Collector, CollectorConfig, SessionEnd};

#[cfg(test)]
mod peer_tests;
#[cfg(test)]
mod tests;
