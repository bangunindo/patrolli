// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Route tables: lookup tables with `source.type: bgp`, written by one BGP/BMP
//! listener.
//!
//! The listener writes through a [`RouteTable`], never through
//! `attribute_collect`: a session's updates must land in the order the TCP
//! stream carried them, and processor workers do not order batches. The rows
//! follow the fixed schema in [`crate::config::lookup_table::bgp`], built from
//! a [`RouteRow`] so the text formats live in one place.

use super::cell::{CellOwned, CellRef, Delete, KeyOwned, KeyRef, canonical};
use super::metrics::TableMetrics;
use super::mutable::{Mutable, Upsert};
pub use super::prefix::Family;
use super::{LookupTable, Table};
use crate::config::lookup_table as cfg;
use anyhow::Result;
use ipnet::IpNet;
use smallvec::SmallVec;
use smol_str::SmolStr;
use std::fmt::Write;
use std::net::IpAddr;
use std::sync::Arc;
use uuid::Uuid;

/// The cells of a route row: every column but the prefix, which the index
/// consumes.
pub const CELLS: usize = cfg::bgp::COLUMNS.len() - 1;

/// One segment of an AS path (RFC 4271 §4.3, RFC 5065 for the confederation
/// kinds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSegment<'a> {
    Sequence(&'a [u32]),
    Set(&'a [u32]),
    ConfedSequence(&'a [u32]),
    ConfedSet(&'a [u32]),
}

/// One route, as the listener hands it to the table.
///
/// Borrowed throughout, so a listener can point into its parsed message
/// without copying. ASNs are 4-byte: merging `AS4_PATH` into a 2-byte
/// `AS_PATH` is the parser's job.
#[derive(Debug, Clone, Copy)]
pub struct RouteRow<'a> {
    pub router: IpAddr,
    /// [`cfg::bgp::GLOBAL_VRF`] for route distinguisher 0
    pub vrf: &'a str,
    pub prefix: IpNet,
    pub next_hop: Option<IpAddr>,
    pub as_path: &'a [PathSegment<'a>],
    /// RFC 1997, each as its 32-bit value
    pub communities: &'a [u32],
    /// RFC 8092: global administrator, local data 1, local data 2
    pub large_communities: &'a [[u32; 3]],
    pub local_pref: Option<u32>,
    pub med: Option<u32>,
    /// RFC 7911 path id, when the router sends them
    pub path_id: Option<u32>,
}

/// Reusable buffers for the text columns of a [`RouteRow`], so rendering a
/// route allocates nothing once they have grown.
#[derive(Debug, Default)]
pub struct RouteText {
    as_path: String,
    communities: String,
    large_communities: String,
}

impl RouteRow<'_> {
    /// The row's cells in table order (see [`cfg::bgp::COLUMNS`]), borrowing
    /// the rendered text from `text`. Empty text columns are null, so a lookup
    /// leaves its destination alone rather than writing `""`.
    pub fn cells<'s>(&'s self, text: &'s mut RouteText) -> [CellRef<'s>; CELLS] {
        render_as_path(self.as_path, &mut text.as_path);
        text.communities.clear();
        for (i, c) in self.communities.iter().enumerate() {
            if i > 0 {
                text.communities.push(' ');
            }
            let _ = write!(text.communities, "{}:{}", c >> 16, c & 0xffff);
        }
        text.large_communities.clear();
        for (i, [global, data1, data2]) in self.large_communities.iter().enumerate() {
            if i > 0 {
                text.large_communities.push(' ');
            }
            let _ = write!(text.large_communities, "{global}:{data1}:{data2}");
        }

        let integer = |v: Option<u32>| v.map_or(CellRef::Null, |v| CellRef::Integer(v.into()));
        let string = |s: &'s str| {
            if s.is_empty() {
                CellRef::Null
            } else {
                CellRef::String(s)
            }
        };
        [
            CellRef::Ip(canonical(self.router)),
            CellRef::String(self.vrf),
            self.next_hop
                .map_or(CellRef::Null, |ip| CellRef::Ip(canonical(ip))),
            integer(origin_as(self.as_path)),
            integer(peer_as(self.as_path)),
            string(&text.as_path),
            string(&text.communities),
            string(&text.large_communities),
            integer(self.local_pref),
            integer(self.med),
            integer(self.path_id),
        ]
    }
}

/// Space separated, a set as `{a,b}`, a confederation sequence as `(a b)` and a
/// confederation set as `[a,b]` — the bgpdump spellings.
fn render_as_path(path: &[PathSegment<'_>], out: &mut String) {
    out.clear();
    for segment in path {
        let (open, sep, close, asns) = match segment {
            PathSegment::Sequence(asns) => ("", ' ', "", asns),
            PathSegment::Set(asns) => ("{", ',', "}", asns),
            PathSegment::ConfedSequence(asns) => ("(", ' ', ")", asns),
            PathSegment::ConfedSet(asns) => ("[", ',', "]", asns),
        };
        if asns.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(open);
        for (i, asn) in asns.iter().enumerate() {
            if i > 0 {
                out.push(sep);
            }
            let _ = write!(out, "{asn}");
        }
        out.push_str(close);
    }
}

/// The segments that count toward the path, without the confederation ones
/// (RFC 5065 §5.3), which never leave the confederation's own view.
fn counted<'a>(
    path: &'a [PathSegment<'a>],
) -> impl DoubleEndedIterator<Item = &'a PathSegment<'a>> {
    path.iter()
        .filter(|s| matches!(s, PathSegment::Sequence(a) | PathSegment::Set(a) if !a.is_empty()))
}

/// The last AS of the path. An aggregate ending in a set has one origin only
/// when the set has one member; otherwise there is no single answer, and null
/// beats picking one.
fn origin_as(path: &[PathSegment<'_>]) -> Option<u32> {
    match counted(path).next_back()? {
        PathSegment::Sequence(asns) => asns.last().copied(),
        PathSegment::Set([only]) => Some(*only),
        _ => None,
    }
}

/// The first AS of the path: the neighbour the route came through. Null for a
/// path that starts with a set, and for an empty path (a route originated
/// inside the router's own AS).
fn peer_as(path: &[PathSegment<'_>]) -> Option<u32> {
    match counted(path).next()? {
        PathSegment::Sequence(asns) => asns.first().copied(),
        _ => None,
    }
}

/// The write handle of one route table.
///
/// It holds the table's rows, not the registry entry, so it notices neither an
/// edit nor a delete on its own: check [`Self::is_current`] once per message
/// batch. A rebuilt table starts empty, and only a resync can fill it again.
#[derive(Debug, Clone)]
pub struct RouteTable {
    id: Uuid,
    table: Arc<Mutable>,
    metrics: Arc<TableMetrics>,
}

impl LookupTable {
    /// The route table `id`, if it is configured and has a `bgp` source.
    pub fn route_table(&self, id: &Uuid) -> Option<RouteTable> {
        let Table::Mutable(table) = self.get(id)? else {
            return None;
        };
        if !matches!(table.meta.config.source, cfg::Source::Bgp(_)) {
            return None;
        }
        Some(RouteTable {
            id: *id,
            table: Arc::clone(&table.table),
            metrics: Arc::clone(&table.meta.metrics),
        })
    }

    /// The route table whose `bgp` source names `listener`. Configuration
    /// allows at most one; should two slip through, the lowest id wins, so
    /// the answer is stable.
    pub fn route_table_for_listener(&self, listener: &Uuid) -> Option<RouteTable> {
        let tables = self.registry.load();
        let id = tables
            .iter()
            .filter(|(_, table)| {
                matches!(
                    &table.meta().config.source,
                    cfg::Source::Bgp(bgp) if bgp.listener == *listener
                )
            })
            .map(|(id, _)| *id)
            .min()?;
        self.route_table(&id)
    }
}

impl RouteTable {
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Whether `registry` still serves the rows this handle writes. False once
    /// the table is removed, or rebuilt by an edit that cannot keep its rows
    /// (a different listener); the writer must then fetch a new handle and
    /// resync every session, since the new table starts empty.
    pub fn is_current(&self, registry: &LookupTable) -> bool {
        matches!(
            registry.get(&self.id),
            Some(Table::Mutable(table)) if Arc::ptr_eq(&table.table, &self.table)
        )
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Adds or replaces `route`. `Rejected` means the table is at `max_rows`.
    pub fn upsert(&self, route: &RouteRow<'_>, text: &mut RouteText) -> Result<Upsert> {
        self.table.upsert_prefix(&route.cells(text), route.prefix)
    }

    /// Removes exactly `prefix` from one router's VRF; a covering or a nested
    /// prefix is left alone.
    pub fn withdraw(&self, router: IpAddr, vrf: &str, prefix: IpNet) -> bool {
        self.table.remove(Delete::Prefix {
            partition: &partition(&router, vrf),
            net: prefix,
        })
    }

    /// Removes every route of one router's VRF: the peer went down.
    pub fn peer_down(&self, router: IpAddr, vrf: &str) -> usize {
        self.table.remove_partition(&partition(&router, vrf))
    }

    /// Removes every route; returns how many went. For a listener gone for
    /// good: nothing would ever withdraw or expire what it wrote.
    pub fn clear(&self) -> usize {
        let removed = self.table.len();
        self.table.clear();
        removed
    }

    /// Every (router, VRF) the table holds routes for.
    pub fn partitions(&self) -> Vec<(IpAddr, SmolStr)> {
        self.table
            .partitions()
            .iter()
            .filter_map(|key| match &key[..] {
                [KeyOwned::Ip(router), KeyOwned::String(vrf)] => Some((*router, vrf.clone())),
                _ => None,
            })
            .collect()
    }

    /// Removes the routes of one router's VRF written before `cutoff`, a
    /// [`super::mutable::now_millis`] taken when the session (re)started,
    /// limited to one address family when `family` is given. Run it after
    /// End-of-RIB: what the router did not re-send is gone. Counted as expired
    /// rows.
    pub fn sweep(&self, router: IpAddr, vrf: &str, family: Option<Family>, cutoff: u64) -> usize {
        let removed = self
            .table
            .sweep_partition(&partition(&router, vrf), family, cutoff);
        self.metrics.note_sweep(removed, 0);
        removed
    }

    /// The path id of the row stored under exactly `prefix` and when the row
    /// was written: `None` when there is no such row, a `None` id when the
    /// row has no path id.
    pub fn shown_path(
        &self,
        router: IpAddr,
        vrf: &str,
        prefix: IpNet,
    ) -> Option<(Option<u32>, u64)> {
        let row = self.table.get_prefix(&partition(&router, vrf), prefix)?;
        let id = match row.cells().nth(PATH_ID_CELL) {
            Some(CellRef::Integer(id)) => u32::try_from(id).ok(),
            _ => None,
        };
        Some((id, row.last_seen()))
    }

    /// The cells of the row stored under exactly `prefix`, owned, with the
    /// time it was written, so another path can take its place and the row be
    /// written back later.
    pub fn row_cells(
        &self,
        router: IpAddr,
        vrf: &str,
        prefix: IpNet,
    ) -> Option<(Box<[CellOwned]>, u64)> {
        let row = self.table.get_prefix(&partition(&router, vrf), prefix)?;
        Some((
            row.cells().map(CellRef::to_owned_cell).collect(),
            row.last_seen(),
        ))
    }

    /// Writes cells taken from [`Self::row_cells`] or rendered earlier by
    /// [`RouteRow::cells`]; the row is stamped now.
    pub fn upsert_cells(&self, cells: &[CellOwned], prefix: IpNet) -> Result<Upsert> {
        let cells: SmallVec<[CellRef<'_>; CELLS]> = cells.iter().map(CellOwned::as_ref).collect();
        self.table.upsert_prefix(&cells, prefix)
    }
}

/// Where [`cfg::bgp::PATH_ID`] sits among a row's cells.
pub const PATH_ID_CELL: usize = CELLS - 1;

fn partition<'a>(router: &IpAddr, vrf: &'a str) -> [KeyRef<'a>; 2] {
    [KeyRef::Ip(canonical(*router)), KeyRef::String(vrf)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::lookup_table::cell::Lookup;
    use crate::cache::lookup_table::mutable::now_millis;
    use std::time::Duration;

    fn route<'a>(router: &str, vrf: &'a str, prefix: &str) -> RouteRow<'a> {
        RouteRow {
            router: router.parse().unwrap(),
            vrf,
            prefix: prefix.parse().unwrap(),
            next_hop: None,
            as_path: &[],
            communities: &[],
            large_communities: &[],
            local_pref: None,
            med: None,
            path_id: None,
        }
    }

    fn bgp_config(listener: Uuid, max_rows: usize) -> cfg::LookupTable {
        cfg::LookupTable {
            id: Uuid::now_v7(),
            name: "routes".into(),
            source: cfg::Source::Bgp(cfg::Bgp { listener, max_rows }),
            key: cfg::Bgp::key(),
        }
    }

    #[test]
    fn cells_follow_the_table_schema() {
        let registry = LookupTable::new();
        let config = bgp_config(Uuid::now_v7(), 10);
        registry.insert(&config).unwrap();
        let read = registry.get(&config.id).unwrap().read();
        let expected: Vec<_> = cfg::bgp::COLUMNS
            .iter()
            .filter(|(name, _)| *name != cfg::bgp::PREFIX)
            .collect();
        assert_eq!(expected.len(), CELLS);
        for (i, (name, ty)) in expected.iter().enumerate() {
            let idx = read.column(name).unwrap();
            assert_eq!(idx as usize, i, "{name} is out of order");
            assert_eq!(read.schema().value_type(idx), Some(*ty));
        }
    }

    #[test]
    fn a_route_renders_its_text_columns() {
        let path = [
            PathSegment::ConfedSequence(&[65001, 65002]),
            PathSegment::Sequence(&[64500, 4_200_000_000]),
            PathSegment::Set(&[64510, 64511]),
        ];
        let row = RouteRow {
            next_hop: Some("::ffff:192.0.2.1".parse().unwrap()),
            as_path: &path,
            communities: &[(64500 << 16) | 100, 0xffff_ff01],
            large_communities: &[[4_200_000_000, 1, 2]],
            local_pref: Some(200),
            med: Some(0),
            ..route("192.0.2.254", "global", "198.51.100.0/24")
        };
        let mut text = RouteText::default();
        let cells = row.cells(&mut text);
        assert_eq!(cells[0], CellRef::Ip("192.0.2.254".parse().unwrap()));
        assert_eq!(cells[1], CellRef::String("global"));
        assert_eq!(
            cells[2],
            CellRef::Ip("192.0.2.1".parse().unwrap()),
            "a v4-mapped next hop is stored as v4"
        );
        assert_eq!(
            cells[3],
            CellRef::Null,
            "a two-member set has no one origin"
        );
        assert_eq!(
            cells[4],
            CellRef::Integer(64500),
            "confed segments are skipped"
        );
        assert_eq!(
            cells[5],
            CellRef::String("(65001 65002) 64500 4200000000 {64510,64511}")
        );
        assert_eq!(cells[6], CellRef::String("64500:100 65535:65281"));
        assert_eq!(cells[7], CellRef::String("4200000000:1:2"));
        assert_eq!(cells[8], CellRef::Integer(200));
        assert_eq!(cells[9], CellRef::Integer(0), "a zero med is not absent");
        assert_eq!(cells[10], CellRef::Null, "no path id without ADD-PATH");
    }

    #[test]
    fn origin_and_peer_as_come_from_the_ends_of_the_path() {
        let seq = [PathSegment::Sequence(&[64500, 64501, 64502])];
        assert_eq!(origin_as(&seq), Some(64502));
        assert_eq!(peer_as(&seq), Some(64500));

        let single_set = [PathSegment::Sequence(&[64500]), PathSegment::Set(&[64999])];
        assert_eq!(origin_as(&single_set), Some(64999));

        let starts_with_set = [PathSegment::Set(&[64500, 64501])];
        assert_eq!(peer_as(&starts_with_set), None);

        let only_confed = [PathSegment::ConfedSequence(&[65001])];
        assert_eq!(origin_as(&only_confed), None);
        assert_eq!(peer_as(&only_confed), None);
    }

    #[test]
    fn an_empty_route_has_null_text_and_attributes() {
        let row = route("2001:db8::1", "64500:1", "2001:db8:100::/48");
        let mut text = RouteText::default();
        let cells = row.cells(&mut text);
        assert_eq!(cells[1], CellRef::String("64500:1"));
        for (i, cell) in cells.iter().enumerate().skip(2) {
            assert_eq!(*cell, CellRef::Null, "cell {i}");
        }
        // the buffers are reused, not appended to
        let path = [PathSegment::Sequence(&[1])];
        let row = RouteRow {
            as_path: &path,
            ..row
        };
        assert_eq!(row.cells(&mut text)[5], CellRef::String("1"));
        assert_eq!(row.cells(&mut text)[5], CellRef::String("1"));
    }

    #[test]
    fn a_route_table_writes_withdraws_and_sweeps() {
        let registry = LookupTable::new();
        let config = bgp_config(Uuid::now_v7(), 10);
        registry.insert(&config).unwrap();
        let routes = registry.route_table(&config.id).unwrap();
        let mut text = RouteText::default();

        let path = [PathSegment::Sequence(&[64500, 64501])];
        let r = |router, vrf, prefix| RouteRow {
            as_path: &path,
            ..route(router, vrf, prefix)
        };
        for (router, vrf, prefix) in [
            ("192.0.2.1", "global", "10.0.0.0/8"),
            ("192.0.2.1", "global", "10.1.0.0/16"),
            ("192.0.2.1", "64500:1", "10.0.0.0/8"),
            ("192.0.2.2", "global", "10.0.0.0/8"),
        ] {
            assert_eq!(
                routes.upsert(&r(router, vrf, prefix), &mut text).unwrap(),
                Upsert::Inserted
            );
        }
        assert_eq!(routes.len(), 4);

        let read = registry.get(&config.id).unwrap().read();
        let origin = read.column(cfg::bgp::ORIGIN_AS).unwrap();
        let router: IpAddr = "192.0.2.1".parse().unwrap();
        let probe = |vrf: &str, addr: &str| {
            read.lookup(Lookup::Prefix {
                partition: &partition(&router, vrf),
                addr: addr.parse().unwrap(),
            })
            .map(|m| {
                (
                    m.prefix.unwrap().to_string(),
                    m.row.cell(origin).to_owned_cell(),
                )
            })
        };
        assert_eq!(
            probe("global", "10.1.2.3").unwrap().0,
            "10.1.0.0/16",
            "longest match"
        );

        assert!(routes.withdraw(router, "global", "10.1.0.0/16".parse().unwrap()));
        assert_eq!(probe("global", "10.1.2.3").unwrap().0, "10.0.0.0/8");
        assert!(!routes.withdraw(router, "global", "10.1.0.0/16".parse().unwrap()));

        // a resync re-sends one route; the sweep removes the rest of that VRF
        std::thread::sleep(Duration::from_millis(2));
        let cutoff = now_millis();
        let _ = routes
            .upsert(&r("192.0.2.1", "64500:1", "10.0.0.0/8"), &mut text)
            .unwrap();
        assert_eq!(routes.sweep(router, "64500:1", None, cutoff), 0);
        assert_eq!(routes.sweep(router, "global", None, cutoff), 1);
        assert_eq!(probe("global", "10.1.2.3"), None);
        assert!(probe("64500:1", "10.1.2.3").is_some());
        assert_eq!(
            registry
                .get(&config.id)
                .unwrap()
                .meta()
                .metrics
                .totals()
                .rows_expired,
            1
        );

        // a peer going down takes the whole VRF, and only it
        assert_eq!(routes.peer_down(router, "64500:1"), 1);
        assert_eq!(routes.len(), 1, "the other router keeps its route");
    }

    #[test]
    fn only_a_bgp_table_hands_out_a_route_handle() {
        let registry = LookupTable::new();
        let processor: cfg::LookupTable = serde_json::from_value(serde_json::json!({
            "id": Uuid::now_v7(),
            "name": "live",
            "source": {"type": "processor", "columns": [{"name": "host"}]},
            "key": {"type": "hash", "key_columns": ["host"]},
        }))
        .unwrap();
        registry.insert(&processor).unwrap();
        assert!(registry.route_table(&processor.id).is_none());
        assert!(registry.route_table(&Uuid::now_v7()).is_none());
    }

    #[test]
    fn a_handle_goes_stale_only_when_the_rows_are_replaced() {
        let registry = LookupTable::new();
        let listener = Uuid::now_v7();
        let mut config = bgp_config(listener, 10);
        registry.upsert(&config).unwrap();
        let routes = registry.route_table(&config.id).unwrap();
        let mut text = RouteText::default();
        let _ = routes
            .upsert(&route("192.0.2.1", "global", "10.0.0.0/8"), &mut text)
            .unwrap();
        assert!(routes.is_current(&registry));

        // a rename and a new cap keep the rows
        config.name = "renamed".into();
        config.source = cfg::Source::Bgp(cfg::Bgp {
            listener,
            max_rows: 20,
        });
        registry.upsert(&config).unwrap();
        assert!(routes.is_current(&registry));
        assert_eq!(registry.route_table(&config.id).unwrap().len(), 1);

        // another listener gets an empty table
        config.source = cfg::Source::Bgp(cfg::Bgp {
            listener: Uuid::now_v7(),
            max_rows: 20,
        });
        registry.upsert(&config).unwrap();
        assert!(!routes.is_current(&registry));
        let fresh = registry.route_table(&config.id).unwrap();
        assert!(fresh.is_empty());
        assert!(fresh.is_current(&registry));

        // and so does a delete and recreate
        registry.remove(&config.id);
        assert!(!fresh.is_current(&registry));
        registry.upsert(&config).unwrap();
        assert!(!fresh.is_current(&registry));
    }
}
