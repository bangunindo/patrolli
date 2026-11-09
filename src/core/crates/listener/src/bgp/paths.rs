// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! ADD-PATH (RFC 7911): several paths per prefix, one row per prefix.
//!
//! The table shows the live path with the lowest path id; its `path_id`
//! column says which one. The other paths wait here, as rendered cells, so
//! withdrawing the shown path promotes the next one instead of deleting a
//! prefix that is still reachable. Only prefixes with more than one path have
//! an entry, so a single-path table costs nothing here.

use anyhow::Result;
use ipnet::IpNet;
use patrolli::cache::lookup_table::bgp::{Family, RouteRow, RouteTable, RouteText};
use patrolli::cache::lookup_table::cell::{CellOwned, CellRef};
use patrolli::cache::lookup_table::mutable::{Upsert, now_millis};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::net::IpAddr;

#[derive(Debug)]
struct Extra {
    path_id: u32,
    /// when this path was last announced, in [`now_millis`] time
    last_seen: u64,
    cells: Box<[CellOwned]>,
}

/// What a withdrawal did to the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Withdrawn {
    /// the prefix is gone
    Removed,
    /// a waiting path replaced the withdrawn one
    Promoted,
    /// a waiting path was dropped; the table is unchanged
    Hidden,
    /// neither the table nor the side map held it
    Unknown,
}

/// The paths of one session that the table does not show.
#[derive(Debug)]
pub struct Paths {
    router: IpAddr,
    /// the session's start: a path last seen before it is not promoted, since
    /// the router has not confirmed it yet
    cutoff: u64,
    extras: FxHashMap<(SmolStr, IpNet), SmallVec<[Extra; 1]>>,
}

impl Paths {
    pub fn new(router: IpAddr, cutoff: u64) -> Self {
        Self {
            router,
            cutoff,
            extras: FxHashMap::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.extras.values().map(SmallVec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.extras.is_empty()
    }

    /// Writes or stores one announced path. Without a path id the route just
    /// replaces the row.
    pub fn announce(
        &mut self,
        table: &RouteTable,
        vrf: &SmolStr,
        route: &RouteRow<'_>,
        text: &mut RouteText,
    ) -> Result<Upsert> {
        let Some(id) = route.path_id else {
            return table.upsert(route, text);
        };
        let prefix = route.prefix;
        match table.shown_path(self.router, vrf, prefix) {
            // a lower path this session confirmed is shown: this one waits.
            // A lower path left by an earlier session may be gone; this one
            // takes the row below, or a sweep would remove the row and leave
            // this path waiting behind nothing.
            Some((Some(shown), last_seen)) if shown < id && last_seen >= self.cutoff => {
                let cells = route
                    .cells(text)
                    .iter()
                    .map(|c| c.to_owned_cell())
                    .collect();
                self.put(vrf, prefix, id, now_millis(), cells);
                Ok(Upsert::Updated)
            }
            // a higher path, or an unconfirmed lower one, is shown: it moves
            // aside, keeping its age, so the sweep drops it unless re-sent
            Some((Some(shown), _)) if shown != id => {
                if let Some((cells, last_seen)) = table.row_cells(self.router, vrf, prefix) {
                    self.put(vrf, prefix, shown, last_seen, cells);
                }
                self.take(vrf, prefix, id);
                table.upsert(route, text)
            }
            // nothing, this path, or a row without a path id: replace it
            _ => {
                self.take(vrf, prefix, id);
                table.upsert(route, text)
            }
        }
    }

    /// Removes one path, promoting the lowest waiting one when the shown path
    /// goes.
    pub fn withdraw(
        &mut self,
        table: &RouteTable,
        vrf: &SmolStr,
        prefix: IpNet,
        path_id: Option<u32>,
    ) -> Result<Withdrawn> {
        let Some(id) = path_id else {
            return Ok(if table.withdraw(self.router, vrf, prefix) {
                Withdrawn::Removed
            } else {
                Withdrawn::Unknown
            });
        };
        match table.shown_path(self.router, vrf, prefix) {
            Some((shown, _)) if shown.is_none_or(|shown| shown == id) => {
                while let Some(next) = self.take_lowest(vrf, prefix) {
                    if next.last_seen < self.cutoff {
                        // not re-announced since the session started: it may
                        // be gone, and promoting it would make it look fresh
                        continue;
                    }
                    let _ = table.upsert_cells(&next.cells, prefix)?;
                    return Ok(Withdrawn::Promoted);
                }
                table.withdraw(self.router, vrf, prefix);
                Ok(Withdrawn::Removed)
            }
            _ => Ok(if self.take(vrf, prefix, id).is_some() {
                Withdrawn::Hidden
            } else {
                Withdrawn::Unknown
            }),
        }
    }

    /// Drops the waiting paths of `vrf` last seen before `cutoff`, limited to
    /// one family when given: the End-of-RIB and `keep` sweeps.
    pub fn sweep(&mut self, vrf: &str, family: Option<Family>, cutoff: u64) {
        self.extras.retain(|(v, net), paths| {
            if v != vrf || family.is_some_and(|f| f != Family::of(net)) {
                return true;
            }
            paths.retain(|p| p.last_seen >= cutoff);
            !paths.is_empty()
        });
    }

    /// Drops every waiting path of `vrf`: its peer went down.
    pub fn drop_vrf(&mut self, vrf: &str) {
        self.extras.retain(|(v, _), _| v != vrf);
    }

    fn put(
        &mut self,
        vrf: &SmolStr,
        prefix: IpNet,
        path_id: u32,
        last_seen: u64,
        cells: Box<[CellOwned]>,
    ) {
        let paths = self.extras.entry((vrf.clone(), prefix)).or_default();
        let extra = Extra {
            path_id,
            last_seen,
            cells,
        };
        match paths.iter_mut().find(|p| p.path_id == path_id) {
            Some(slot) => *slot = extra,
            None => paths.push(extra),
        }
    }

    fn take(&mut self, vrf: &SmolStr, prefix: IpNet, path_id: u32) -> Option<Extra> {
        let key = (vrf.clone(), prefix);
        let paths = self.extras.get_mut(&key)?;
        let at = paths.iter().position(|p| p.path_id == path_id)?;
        let extra = paths.swap_remove(at);
        if paths.is_empty() {
            self.extras.remove(&key);
        }
        Some(extra)
    }

    fn take_lowest(&mut self, vrf: &SmolStr, prefix: IpNet) -> Option<Extra> {
        let lowest = self
            .extras
            .get(&(vrf.clone(), prefix))?
            .iter()
            .map(|p| p.path_id)
            .min()?;
        self.take(vrf, prefix, lowest)
    }
}

/// The path id a set of rendered cells carries, for tests and debugging.
pub fn cells_path_id(cells: &[CellOwned]) -> Option<u32> {
    use patrolli::cache::lookup_table::bgp::PATH_ID_CELL;
    match cells.get(PATH_ID_CELL).map(CellOwned::as_ref) {
        Some(CellRef::Integer(id)) => u32::try_from(id).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patrolli::cache::lookup_table::LookupTable;
    use patrolli::cache::lookup_table::bgp::PathSegment;
    use patrolli::config::lookup_table as cfg;
    use std::time::Duration;
    use uuid::Uuid;

    const ROUTER: &str = "192.0.2.1";

    struct Fixture {
        registry: LookupTable,
        table: RouteTable,
        text: RouteText,
        vrf: SmolStr,
    }

    impl Fixture {
        fn new() -> Self {
            let registry = LookupTable::new();
            let config = cfg::LookupTable {
                id: Uuid::now_v7(),
                name: "routes".into(),
                source: cfg::Source::Bgp(cfg::Bgp {
                    listener: Uuid::now_v7(),
                    max_rows: 100,
                }),
                key: cfg::Bgp::key(),
            };
            registry.insert(&config).unwrap();
            let table = registry.route_table(&config.id).unwrap();
            Self {
                registry,
                table,
                text: RouteText::default(),
                vrf: SmolStr::new(cfg::bgp::GLOBAL_VRF),
            }
        }

        fn announce(&mut self, paths: &mut Paths, prefix: &str, id: u32, asn: u32) {
            let path = [PathSegment::Sequence(&[asn])];
            let route = RouteRow {
                router: ROUTER.parse().unwrap(),
                vrf: &self.vrf,
                prefix: prefix.parse().unwrap(),
                next_hop: None,
                as_path: &path,
                communities: &[],
                large_communities: &[],
                local_pref: None,
                med: None,
                path_id: Some(id),
            };
            let vrf = self.vrf.clone();
            let _ = paths
                .announce(&self.table, &vrf, &route, &mut self.text)
                .unwrap();
        }

        fn withdraw(&mut self, paths: &mut Paths, prefix: &str, id: u32) -> Withdrawn {
            paths
                .withdraw(&self.table, &self.vrf, prefix.parse().unwrap(), Some(id))
                .unwrap()
        }

        /// (path id, origin AS) of the row the table shows for `prefix`.
        fn shown(&self, prefix: &str) -> Option<(Option<u32>, i64)> {
            let (cells, _) = self.table.row_cells(
                ROUTER.parse().unwrap(),
                &self.vrf,
                prefix.parse().unwrap(),
            )?;
            let read = self.registry.get(&self.table.id()).unwrap().read();
            let origin = read.column(cfg::bgp::ORIGIN_AS).unwrap() as usize;
            let CellOwned::Integer(asn) = cells[origin] else {
                panic!("no origin")
            };
            Some((cells_path_id(&cells), asn))
        }
    }

    fn paths(cutoff: u64) -> Paths {
        Paths::new(ROUTER.parse().unwrap(), cutoff)
    }

    #[test]
    fn the_lowest_path_is_shown_and_the_next_one_promoted() {
        let mut f = Fixture::new();
        let mut p = paths(0);
        f.announce(&mut p, "10.0.0.0/8", 2, 64502);
        assert_eq!(f.shown("10.0.0.0/8"), Some((Some(2), 64502)));
        assert!(p.is_empty(), "a single path needs no side entry");

        // a lower path takes the row, the higher one waits
        f.announce(&mut p, "10.0.0.0/8", 1, 64501);
        assert_eq!(f.shown("10.0.0.0/8"), Some((Some(1), 64501)));
        assert_eq!(p.len(), 1);
        // a higher path waits without touching the row
        f.announce(&mut p, "10.0.0.0/8", 3, 64503);
        assert_eq!(f.shown("10.0.0.0/8"), Some((Some(1), 64501)));
        assert_eq!(p.len(), 2);
        // a waiting path is updated in place
        f.announce(&mut p, "10.0.0.0/8", 3, 64513);
        assert_eq!(p.len(), 2);

        // withdrawing the shown path promotes the lowest waiting one
        assert_eq!(f.withdraw(&mut p, "10.0.0.0/8", 1), Withdrawn::Promoted);
        assert_eq!(f.shown("10.0.0.0/8"), Some((Some(2), 64502)));
        // withdrawing a waiting path leaves the row alone
        assert_eq!(f.withdraw(&mut p, "10.0.0.0/8", 3), Withdrawn::Hidden);
        assert_eq!(f.shown("10.0.0.0/8"), Some((Some(2), 64502)));
        assert!(p.is_empty());
        // and the last one takes the prefix with it
        assert_eq!(f.withdraw(&mut p, "10.0.0.0/8", 2), Withdrawn::Removed);
        assert_eq!(f.shown("10.0.0.0/8"), None);
        assert_eq!(f.withdraw(&mut p, "10.0.0.0/8", 2), Withdrawn::Unknown);
    }

    #[test]
    fn a_reconnect_dump_corrects_rows_of_the_previous_session() {
        let mut f = Fixture::new();
        // the previous session showed path 2 only
        let mut old = paths(0);
        f.announce(&mut old, "10.0.0.0/8", 2, 64502);

        std::thread::sleep(Duration::from_millis(2));
        let cutoff = now_millis();
        let mut p = paths(cutoff);
        // the new dump starts with path 1: path 2's old row moves aside, aged
        f.announce(&mut p, "10.0.0.0/8", 1, 64501);
        assert_eq!(f.shown("10.0.0.0/8"), Some((Some(1), 64501)));
        // path 1 goes before path 2 is re-sent: the unconfirmed path 2 is not
        // promoted
        assert_eq!(f.withdraw(&mut p, "10.0.0.0/8", 1), Withdrawn::Removed);
        assert_eq!(f.shown("10.0.0.0/8"), None);
        assert!(p.is_empty());
    }

    #[test]
    fn a_reconnect_dump_without_the_old_lowest_path_keeps_the_prefix() {
        let mut f = Fixture::new();
        let router: IpAddr = ROUTER.parse().unwrap();
        // the previous session showed path 1, path 2 waiting
        let mut old = paths(0);
        f.announce(&mut old, "10.0.0.0/8", 1, 64501);
        f.announce(&mut old, "10.0.0.0/8", 2, 64502);
        f.announce(&mut old, "10.1.0.0/16", 1, 64501);

        // path 1 went while disconnected: the new dump re-sends only path 2
        std::thread::sleep(Duration::from_millis(2));
        let cutoff = now_millis();
        let mut p = paths(cutoff);
        f.announce(&mut p, "10.0.0.0/8", 2, 64502);
        assert_eq!(
            f.shown("10.0.0.0/8"),
            Some((Some(2), 64502)),
            "a fresh path takes the row from an unconfirmed lower one"
        );
        // End-of-RIB: the unconfirmed path 1 goes, path 2 stays shown
        assert_eq!(f.table.sweep(router, &f.vrf, None, cutoff), 1);
        p.sweep(&f.vrf, None, cutoff);
        assert_eq!(f.shown("10.0.0.0/8"), Some((Some(2), 64502)));
        assert_eq!(f.shown("10.1.0.0/16"), None, "a prefix not re-sent goes");
        assert!(p.is_empty());

        // had path 1 been re-sent after all, it takes the row back
        f.announce(&mut old, "10.2.0.0/16", 1, 64501);
        std::thread::sleep(Duration::from_millis(2));
        let mut p = paths(now_millis());
        f.announce(&mut p, "10.2.0.0/16", 2, 64502);
        f.announce(&mut p, "10.2.0.0/16", 1, 64501);
        assert_eq!(f.shown("10.2.0.0/16"), Some((Some(1), 64501)));
        assert_eq!(p.len(), 1, "path 2 waits again");
    }

    #[test]
    fn a_sweep_drops_waiting_paths_the_dump_did_not_refresh() {
        let mut f = Fixture::new();
        let mut p = paths(0);
        f.announce(&mut p, "10.0.0.0/8", 1, 64501);
        f.announce(&mut p, "10.0.0.0/8", 2, 64502);
        f.announce(&mut p, "2001:db8::/32", 1, 64501);
        f.announce(&mut p, "2001:db8::/32", 2, 64502);
        assert_eq!(p.len(), 2);
        std::thread::sleep(Duration::from_millis(2));
        let cutoff = now_millis();
        p.sweep(&f.vrf, Some(Family::V6), cutoff);
        assert_eq!(p.len(), 1, "only v6 is swept");
        p.sweep("64500:1", None, cutoff);
        assert_eq!(p.len(), 1, "another vrf is untouched");
        p.sweep(&f.vrf, None, cutoff);
        assert!(p.is_empty());
    }

    #[test]
    fn without_path_ids_a_route_replaces_the_row() {
        let mut f = Fixture::new();
        let mut p = paths(0);
        let route = RouteRow {
            router: ROUTER.parse().unwrap(),
            vrf: "global",
            prefix: "10.0.0.0/8".parse().unwrap(),
            next_hop: None,
            as_path: &[],
            communities: &[],
            large_communities: &[],
            local_pref: None,
            med: None,
            path_id: None,
        };
        let vrf = f.vrf.clone();
        assert_eq!(
            p.announce(&f.table, &vrf, &route, &mut f.text).unwrap(),
            Upsert::Inserted
        );
        assert_eq!(
            p.withdraw(&f.table, &vrf, route.prefix, None).unwrap(),
            Withdrawn::Removed
        );
        assert_eq!(
            p.withdraw(&f.table, &vrf, route.prefix, None).unwrap(),
            Withdrawn::Unknown
        );
    }
}
