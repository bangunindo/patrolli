// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! BGP route collection: BMP sessions (RFC 7854, Loc-RIB per RFC 9069) that
//! write the routes into a `bgp` lookup table (see `docs/bgp-bmp.md`).

pub mod codec;
pub mod events;
pub mod listener;
pub mod paths;
pub mod routers;
pub mod session;
pub mod stats;
pub mod wire;

pub use listener::BgpListener;
pub use session::{BmpConfig, Collector, SessionEnd};

#[cfg(test)]
mod tests;
