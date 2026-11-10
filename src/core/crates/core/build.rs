// SPDX-FileCopyrightText: 2025 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

fn main() {
    // `embed_migrations!` (src/db/mod.rs) reads this directory when it expands,
    // which cargo does not track: without this, a new migration is left out of
    // the binary until something else rebuilds the crate.
    println!("cargo:rerun-if-changed=migrations");
}
