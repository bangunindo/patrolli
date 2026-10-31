// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use anyhow::Result;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub id: Uuid,
    pub name: SmolStr,
    pub timezone: Option<SmolStr>,
}

impl Host {
    pub fn validate(&mut self) -> Result<()> {
        Ok(())
    }
}
