// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::cell::{KeyOwned, KeyRef, RangePoint, hash_key, keys_eq};
use super::schema::RowId;
use crate::config::lookup_table::RangeType;
use hashbrown::HashTable;
use rangemap::RangeInclusiveMap;
use std::net::IpAddr;

/// Why a range was not indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeReject {
    /// the bounds are reversed, or exclusive bounds left nothing between them
    Empty,
    /// the range collides with one already indexed; ranges must be disjoint
    Overlap,
    /// start and end are different IP families, or do not match `range_type`
    Mismatch,
}

/// A key type that can be a range bound: totally ordered and steppable, so
/// exclusive bounds can be folded into inclusive ones at build time.
pub trait RangeKey: Ord + Copy + rangemap::StepLite {
    fn next_up(self) -> Option<Self>;
    fn next_down(self) -> Option<Self>;
}

macro_rules! impl_range_key {
    ($($t:ty)*) => ($(
        impl RangeKey for $t {
            #[inline]
            fn next_up(self) -> Option<Self> { self.checked_add(1) }
            #[inline]
            fn next_down(self) -> Option<Self> { self.checked_sub(1) }
        }
    )*)
}
impl_range_key!(i64 u32 u128);

/// Folds `[start, end]` with arbitrary inclusivity into a closed `[lo, hi]`.
pub(super) fn normalize<K: RangeKey>(
    start: K,
    start_inclusive: bool,
    end: K,
    end_inclusive: bool,
) -> Option<(K, K)> {
    let lo = if start_inclusive {
        start
    } else {
        start.next_up()?
    };
    let hi = if end_inclusive { end } else { end.next_down()? };
    (lo <= hi).then_some((lo, hi))
}

/// Disjoint ranges frozen into three parallel arrays. Lookup is a binary search
/// over `starts` plus one bound check — faster and far more compact than keeping
/// the build-time `RangeInclusiveMap`: 12 bytes per v4 range against roughly 30
/// for a BTreeMap node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranges<K> {
    starts: Box<[K]>,
    ends: Box<[K]>,
    rows: Box<[RowId]>,
}

impl<K> Default for Ranges<K> {
    fn default() -> Self {
        Self {
            starts: Box::new([]),
            ends: Box::new([]),
            rows: Box::new([]),
        }
    }
}

impl<K: Ord + Copy> Ranges<K> {
    #[inline]
    pub fn get(&self, point: K) -> Option<RowId> {
        let i = self.starts.partition_point(|s| *s <= point);
        (i > 0 && point <= self.ends[i - 1]).then(|| self.rows[i - 1])
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Accumulates ranges, rejecting overlaps, then freezes into [`Ranges`].
/// `rangemap` owns the fiddly part: deciding whether a candidate touches what
/// is stored, and keeping the set ordered and disjoint.
#[derive(Debug)]
pub struct RangeBuilder<K: RangeKey> {
    map: RangeInclusiveMap<K, RowId>,
}

impl<K: RangeKey> Default for RangeBuilder<K> {
    fn default() -> Self {
        Self {
            map: RangeInclusiveMap::new(),
        }
    }
}

impl<K: RangeKey> RangeBuilder<K> {
    pub fn insert(
        &mut self,
        start: K,
        start_inclusive: bool,
        end: K,
        end_inclusive: bool,
        row: RowId,
    ) -> Result<(), RangeReject> {
        let (lo, hi) =
            normalize(start, start_inclusive, end, end_inclusive).ok_or(RangeReject::Empty)?;
        if self.map.overlapping(&(lo..=hi)).next().is_some() {
            return Err(RangeReject::Overlap);
        }
        self.map.insert(lo..=hi, row);
        Ok(())
    }

    pub fn finish(self) -> Ranges<K> {
        let n = self.map.len();
        let mut starts = Vec::with_capacity(n);
        let mut ends = Vec::with_capacity(n);
        let mut rows = Vec::with_capacity(n);
        // `RangeInclusiveMap` iterates in ascending, disjoint order
        for (range, row) in self.map {
            starts.push(*range.start());
            ends.push(*range.end());
            rows.push(row);
        }
        Ranges {
            starts: starts.into_boxed_slice(),
            ends: ends.into_boxed_slice(),
            rows: rows.into_boxed_slice(),
        }
    }
}

/// The ranges of one partition. An IP table keeps a set per address family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeSet {
    Integer(Ranges<i64>),
    Ip { v4: Ranges<u32>, v6: Ranges<u128> },
}

impl RangeSet {
    #[inline]
    pub fn get(&self, point: RangePoint) -> Option<RowId> {
        match (self, point) {
            (RangeSet::Integer(r), RangePoint::Integer(v)) => r.get(v),
            (RangeSet::Ip { v4, .. }, RangePoint::Ip(IpAddr::V4(v))) => r_get_v4(v4, v),
            (RangeSet::Ip { v6, .. }, RangePoint::Ip(IpAddr::V6(v))) => v6.get(v.to_bits()),
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            RangeSet::Integer(r) => r.len(),
            RangeSet::Ip { v4, v6 } => v4.len() + v6.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[inline]
fn r_get_v4(ranges: &Ranges<u32>, addr: std::net::Ipv4Addr) -> Option<RowId> {
    ranges.get(addr.to_bits())
}

#[derive(Debug)]
pub enum RangeSetBuilder {
    Integer(RangeBuilder<i64>),
    Ip {
        v4: RangeBuilder<u32>,
        v6: RangeBuilder<u128>,
    },
}

impl RangeSetBuilder {
    pub fn new(range_type: RangeType) -> Self {
        match range_type {
            RangeType::Integer => RangeSetBuilder::Integer(RangeBuilder::default()),
            RangeType::Ip => RangeSetBuilder::Ip {
                v4: RangeBuilder::default(),
                v6: RangeBuilder::default(),
            },
        }
    }

    pub fn insert(
        &mut self,
        start: RangePoint,
        start_inclusive: bool,
        end: RangePoint,
        end_inclusive: bool,
        row: RowId,
    ) -> Result<(), RangeReject> {
        match (self, start, end) {
            (RangeSetBuilder::Integer(b), RangePoint::Integer(s), RangePoint::Integer(e)) => {
                b.insert(s, start_inclusive, e, end_inclusive, row)
            }
            (
                RangeSetBuilder::Ip { v4, .. },
                RangePoint::Ip(IpAddr::V4(s)),
                RangePoint::Ip(IpAddr::V4(e)),
            ) => v4.insert(
                s.to_bits(),
                start_inclusive,
                e.to_bits(),
                end_inclusive,
                row,
            ),
            (
                RangeSetBuilder::Ip { v6, .. },
                RangePoint::Ip(IpAddr::V6(s)),
                RangePoint::Ip(IpAddr::V6(e)),
            ) => v6.insert(
                s.to_bits(),
                start_inclusive,
                e.to_bits(),
                end_inclusive,
                row,
            ),
            _ => Err(RangeReject::Mismatch),
        }
    }

    pub fn finish(self) -> RangeSet {
        match self {
            RangeSetBuilder::Integer(b) => RangeSet::Integer(b.finish()),
            RangeSetBuilder::Ip { v4, v6 } => RangeSet::Ip {
                v4: v4.finish(),
                v6: v6.finish(),
            },
        }
    }
}

/// Maps a partition key tuple to its slot. An unpartitioned table has exactly
/// one slot and never hashes anything.
#[derive(Debug, Clone)]
pub struct RangeIndex {
    table: HashTable<u32>,
    keys: Box<[Box<[KeyOwned]>]>,
    sets: Box<[RangeSet]>,
    partitioned: bool,
}

impl RangeIndex {
    #[inline]
    pub fn get(&self, partition: &[KeyRef<'_>], point: RangePoint) -> Option<RowId> {
        let slot = if self.partitioned {
            let hash = hash_key(partition.iter().copied());
            *self
                .table
                .find(hash, |slot| keys_eq(&self.keys[*slot as usize], partition))?
        } else {
            0
        };
        self.sets.get(slot as usize)?.get(point)
    }

    pub fn partitions(&self) -> usize {
        self.sets.len()
    }

    pub fn len(&self) -> usize {
        self.sets.iter().map(RangeSet::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.iter().all(RangeSet::is_empty)
    }
}

#[derive(Debug)]
pub struct RangeIndexBuilder {
    range_type: RangeType,
    partitioned: bool,
    table: HashTable<u32>,
    keys: Vec<Box<[KeyOwned]>>,
    sets: Vec<RangeSetBuilder>,
}

impl RangeIndexBuilder {
    pub fn new(range_type: RangeType, partitioned: bool) -> Self {
        let mut this = Self {
            range_type,
            partitioned,
            table: HashTable::new(),
            keys: Vec::new(),
            sets: Vec::new(),
        };
        if !partitioned {
            this.keys.push(Box::new([]));
            this.sets.push(RangeSetBuilder::new(range_type));
        }
        this
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &mut self,
        partition: &[KeyRef<'_>],
        start: RangePoint,
        start_inclusive: bool,
        end: RangePoint,
        end_inclusive: bool,
        row: RowId,
    ) -> Result<(), RangeReject> {
        if !self.partitioned {
            return self.sets[0].insert(start, start_inclusive, end, end_inclusive, row);
        }
        let hash = hash_key(partition.iter().copied());
        if let Some(slot) = self
            .table
            .find(hash, |slot| keys_eq(&self.keys[*slot as usize], partition))
        {
            return self.sets[*slot as usize].insert(
                start,
                start_inclusive,
                end,
                end_inclusive,
                row,
            );
        }
        // A rejected first interval must not retain an empty partition or its key.
        let mut set = RangeSetBuilder::new(self.range_type);
        set.insert(start, start_inclusive, end, end_inclusive, row)?;
        let slot = self.sets.len() as u32;
        self.keys
            .push(partition.iter().map(|k| k.to_owned_key()).collect());
        self.sets.push(set);
        self.table.insert_unique(hash, slot, |slot| {
            hash_key(self.keys[*slot as usize].iter().map(KeyOwned::as_ref))
        });
        Ok(())
    }

    pub fn finish(self) -> RangeIndex {
        RangeIndex {
            table: self.table,
            keys: self.keys.into_boxed_slice(),
            sets: self
                .sets
                .into_iter()
                .map(RangeSetBuilder::finish)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            partitioned: self.partitioned,
        }
    }
}

/// Exact-match index. Stores only row ids: equality is decided by the caller,
/// which compares the probe against the key columns already in the table.
#[derive(Debug, Clone, Default)]
pub struct HashIndex {
    table: HashTable<RowId>,
}

impl HashIndex {
    #[inline]
    pub fn find(&self, hash: u64, mut eq: impl FnMut(RowId) -> bool) -> Option<RowId> {
        self.table.find(hash, |row| eq(*row)).copied()
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Inserts unless an equal key is already present.
    ///
    /// `hash_of` must return the same hash the row was inserted under; it is
    /// only consulted when the table grows.
    pub fn insert_unique(
        &mut self,
        hash: u64,
        row: RowId,
        mut eq: impl FnMut(RowId) -> bool,
        hash_of: impl Fn(RowId) -> u64,
    ) -> bool {
        if self.table.find(hash, |r| eq(*r)).is_some() {
            return false;
        }
        self.table.insert_unique(hash, row, |r| hash_of(*r));
        true
    }
}

/// The index of a table, one variant per key kind.
#[derive(Debug, Clone)]
pub enum Index {
    Hash(HashIndex),
    Range(RangeIndex),
}

impl Index {
    pub fn len(&self) -> usize {
        match self {
            Index::Hash(h) => h.len(),
            Index::Range(r) => r.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(entries: &[(i64, i64, RowId)]) -> Ranges<i64> {
        let mut b = RangeBuilder::default();
        for (lo, hi, row) in entries {
            b.insert(*lo, true, *hi, true, *row).unwrap();
        }
        b.finish()
    }

    #[test]
    fn point_lookup_hits_inside_and_misses_in_gaps() {
        let r = ranges(&[(10, 20, 1), (30, 40, 2)]);
        assert_eq!(r.get(10), Some(1));
        assert_eq!(r.get(15), Some(1));
        assert_eq!(r.get(20), Some(1));
        assert_eq!(r.get(30), Some(2));
        assert_eq!(r.get(40), Some(2));
        assert_eq!(r.get(9), None, "below the first range");
        assert_eq!(r.get(25), None, "in the gap");
        assert_eq!(r.get(41), None, "above the last range");
    }

    #[test]
    fn empty_set_never_matches() {
        let r: Ranges<i64> = Ranges::default();
        assert_eq!(r.get(0), None);
    }

    #[test]
    fn all_four_inclusivity_combinations() {
        // the source range is 10..20 with varying bound inclusivity
        let cases = [
            (true, true, 10i64, 20i64),
            (true, false, 10, 19),
            (false, true, 11, 20),
            (false, false, 11, 19),
        ];
        for (start_incl, end_incl, want_lo, want_hi) in cases {
            let mut b = RangeBuilder::default();
            b.insert(10i64, start_incl, 20, end_incl, 1).unwrap();
            let r = b.finish();
            assert_eq!(r.get(want_lo), Some(1), "{start_incl}/{end_incl} lo");
            assert_eq!(r.get(want_hi), Some(1), "{start_incl}/{end_incl} hi");
            assert_eq!(r.get(want_lo - 1), None, "{start_incl}/{end_incl} below lo");
            assert_eq!(r.get(want_hi + 1), None, "{start_incl}/{end_incl} above hi");
        }
    }

    #[test]
    fn empty_and_reversed_ranges_are_rejected() {
        let mut b = RangeBuilder::default();
        // exclusive on both sides with nothing between
        assert_eq!(
            b.insert(10i64, false, 11, false, 1),
            Err(RangeReject::Empty)
        );
        // reversed
        assert_eq!(b.insert(20i64, true, 10, true, 1), Err(RangeReject::Empty));
        // exclusive start at the top of the domain has no successor
        assert_eq!(
            b.insert(i64::MAX, false, i64::MAX, true, 1),
            Err(RangeReject::Empty)
        );
        assert_eq!(b.finish().len(), 0);
    }

    #[test]
    fn overlaps_are_rejected_and_leave_the_first_range_intact() {
        let mut b = RangeBuilder::default();
        b.insert(10i64, true, 20, true, 1).unwrap();
        assert_eq!(
            b.insert(15i64, true, 25, true, 2),
            Err(RangeReject::Overlap)
        );
        assert_eq!(
            b.insert(20i64, true, 20, true, 3),
            Err(RangeReject::Overlap)
        );
        assert_eq!(b.insert(5i64, true, 30, true, 4), Err(RangeReject::Overlap));
        // adjacent but not overlapping is fine
        b.insert(21i64, true, 30, true, 5).unwrap();
        let r = b.finish();
        assert_eq!(r.get(15), Some(1));
        assert_eq!(r.get(25), Some(5));
    }

    #[test]
    fn ranges_touching_the_domain_boundaries_do_not_overflow() {
        // a v4 geoip table really does end at 255.255.255.255
        let mut b = RangeBuilder::default();
        b.insert(0u32, true, 100, true, 1).unwrap();
        b.insert(u32::MAX - 10, true, u32::MAX, true, 2).unwrap();
        let r = b.finish();
        assert_eq!(r.get(0), Some(1));
        assert_eq!(r.get(u32::MAX), Some(2));
        assert_eq!(r.get(u32::MAX - 11), None);
    }

    #[test]
    fn ip_sets_keep_the_families_apart() {
        let mut b = RangeSetBuilder::new(RangeType::Ip);
        let v4 = |s: &str| RangePoint::Ip(s.parse().unwrap());
        b.insert(v4("10.0.0.0"), true, v4("10.0.0.255"), true, 1)
            .unwrap();
        b.insert(v4("2001:db8::"), true, v4("2001:db8::ffff"), true, 2)
            .unwrap();
        // a mixed-family range is nonsense
        assert_eq!(
            b.insert(v4("10.0.0.0"), true, v4("2001:db8::"), true, 3),
            Err(RangeReject::Mismatch)
        );
        let set = b.finish();
        assert_eq!(set.get(v4("10.0.0.7")), Some(1));
        assert_eq!(set.get(v4("2001:db8::5")), Some(2));
        assert_eq!(set.get(v4("11.0.0.1")), None);
        assert_eq!(set.get(RangePoint::Integer(5)), None);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn integer_sets_reject_ip_bounds() {
        let mut b = RangeSetBuilder::new(RangeType::Integer);
        assert_eq!(
            b.insert(
                RangePoint::Ip("10.0.0.1".parse().unwrap()),
                true,
                RangePoint::Ip("10.0.0.2".parse().unwrap()),
                true,
                1
            ),
            Err(RangeReject::Mismatch)
        );
    }

    #[test]
    fn partitions_isolate_identical_ranges() {
        let mut b = RangeIndexBuilder::new(RangeType::Integer, true);
        let a = [KeyRef::String("vrf-a")];
        let c = [KeyRef::String("vrf-c")];
        b.insert(
            &a,
            RangePoint::Integer(1),
            true,
            RangePoint::Integer(10),
            true,
            1,
        )
        .unwrap();
        // the same range in another partition is not an overlap
        b.insert(
            &c,
            RangePoint::Integer(1),
            true,
            RangePoint::Integer(10),
            true,
            2,
        )
        .unwrap();
        // but a collision inside one partition still is
        assert_eq!(
            b.insert(
                &a,
                RangePoint::Integer(5),
                true,
                RangePoint::Integer(6),
                true,
                3
            ),
            Err(RangeReject::Overlap)
        );
        let index = b.finish();
        assert_eq!(index.partitions(), 2);
        assert_eq!(index.len(), 2);
        assert_eq!(index.get(&a, RangePoint::Integer(5)), Some(1));
        assert_eq!(index.get(&c, RangePoint::Integer(5)), Some(2));
        assert_eq!(
            index.get(&[KeyRef::String("vrf-z")], RangePoint::Integer(5)),
            None
        );
    }

    #[test]
    fn an_unpartitioned_index_has_a_single_slot() {
        let mut b = RangeIndexBuilder::new(RangeType::Integer, false);
        b.insert(
            &[],
            RangePoint::Integer(1),
            true,
            RangePoint::Integer(10),
            true,
            7,
        )
        .unwrap();
        let index = b.finish();
        assert_eq!(index.partitions(), 1);
        assert_eq!(index.get(&[], RangePoint::Integer(3)), Some(7));
    }

    #[test]
    fn hash_index_rejects_duplicate_keys() {
        // rows 0 and 2 share a key; row 1 differs
        let keys = ["a", "b", "a"];
        let mut index = HashIndex::default();
        let hash_of = |r: RowId| hash_key([KeyRef::String(keys[r as usize])]);
        let mut inserted = Vec::new();
        for row in 0..keys.len() as RowId {
            let h = hash_of(row);
            let ok = index.insert_unique(
                h,
                row,
                |other| keys[other as usize] == keys[row as usize],
                hash_of,
            );
            if ok {
                inserted.push(row);
            }
        }
        assert_eq!(inserted, vec![0, 1]);
        assert_eq!(index.len(), 2);
        assert_eq!(
            index.find(hash_of(0), |r| keys[r as usize] == "a"),
            Some(0),
            "the first row wins"
        );
        assert_eq!(
            index.find(hash_key([KeyRef::String("zz")]), |_| false),
            None
        );
    }
}
