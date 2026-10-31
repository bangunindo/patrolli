// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use crate::cache::item::Stats;
use crate::config::host::Host;
use crate::config::item::{Config as ItemConfig, Item, ItemKind};
use std::mem::discriminant;
use std::sync::Arc;

pub fn generate_data(
    item: &Item,
    host: &Host,
    stats: &Arc<Stats>,
) -> Result<super::Collected, super::Error> {
    let ItemConfig::Dummy(item_conf) = &item.config else {
        return Err(super::Error::config(format!(
            "not a dummy config: {:?}",
            discriminant(&item.config),
        )));
    };
    let resource = super::create_resource(host, None, item);
    let mut result = Vec::with_capacity(item_conf.count);
    for _ in 0..item_conf.count {
        result.push(super::MonitorLog {
            row: item_conf.payload.row.clone(),
            resource: resource.clone(),
            kind: ItemKind::from(&item.config),
            processors: item.processors.clone(),
            stats: stats.clone(),
        });
    }
    let rows = result.len() as u64;
    Ok(super::Collected::new(
        super::MonitorRecord::BatchLog(result),
        rows,
    ))
}
