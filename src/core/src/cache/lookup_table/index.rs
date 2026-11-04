// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

use super::cell::{KeyOwned, KeyRef, hash_key, keys_eq};
use super::prefix::PrefixSet;
use super::schema::RowId;
use hashbrown::HashTable;
use ipnet::IpNet;
use std::net::IpAddr;

/// Why a prefix was not indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixReject {
    /// the exact prefix is already indexed in this partition
    Duplicate,
}

/// Maps a partition key tuple to its slot, each slot a longest-prefix-match
/// set of row ids. An unpartitioned table has exactly one slot and never
/// hashes anything.
#[derive(Debug, Clone)]
pub struct PrefixIndex {
    table: HashTable<u32>,
    keys: Box<[Box<[KeyOwned]>]>,
    sets: Box<[PrefixSet<RowId>]>,
    partitioned: bool,
}

impl PrefixIndex {
    /// The row of the narrowest prefix containing `addr`, and that prefix.
    #[inline]
    pub fn get(&self, partition: &[KeyRef<'_>], addr: IpAddr) -> Option<(RowId, IpNet)> {
        let slot = if self.partitioned {
            let hash = hash_key(partition.iter().copied());
            *self
                .table
                .find(hash, |slot| keys_eq(&self.keys[*slot as usize], partition))?
        } else {
            0
        };
        let (net, row) = self.sets.get(slot as usize)?.lpm(addr)?;
        Some((*row, net))
    }

    pub fn partitions(&self) -> usize {
        self.sets.len()
    }

    pub fn len(&self) -> usize {
        self.sets.iter().map(PrefixSet::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.iter().all(PrefixSet::is_empty)
    }

    /// Bytes held by the tries and the partition table.
    pub fn mem_size(&self) -> usize {
        self.sets.iter().map(PrefixSet::mem_size).sum::<usize>()
            + self.table.capacity() * std::mem::size_of::<u32>()
    }
}

#[derive(Debug)]
pub struct PrefixIndexBuilder {
    partitioned: bool,
    table: HashTable<u32>,
    keys: Vec<Box<[KeyOwned]>>,
    sets: Vec<PrefixSet<RowId>>,
}

impl PrefixIndexBuilder {
    pub fn new(partitioned: bool) -> Self {
        let mut this = Self {
            partitioned,
            table: HashTable::new(),
            keys: Vec::new(),
            sets: Vec::new(),
        };
        if !partitioned {
            this.keys.push(Box::new([]));
            this.sets.push(PrefixSet::new());
        }
        this
    }

    pub fn insert(
        &mut self,
        partition: &[KeyRef<'_>],
        net: IpNet,
        row: RowId,
    ) -> Result<(), PrefixReject> {
        let set = if !self.partitioned {
            &mut self.sets[0]
        } else {
            let hash = hash_key(partition.iter().copied());
            match self
                .table
                .find(hash, |slot| keys_eq(&self.keys[*slot as usize], partition))
            {
                Some(slot) => &mut self.sets[*slot as usize],
                None => {
                    // a new partition always takes its first prefix, so no
                    // rejected row can leave an empty partition behind
                    let slot = self.sets.len() as u32;
                    self.keys
                        .push(partition.iter().map(|k| k.to_owned_key()).collect());
                    self.sets.push(PrefixSet::new());
                    self.table.insert_unique(hash, slot, |slot| {
                        hash_key(self.keys[*slot as usize].iter().map(KeyOwned::as_ref))
                    });
                    self.sets.last_mut().expect("just pushed")
                }
            }
        };
        if set.contains(&net) {
            return Err(PrefixReject::Duplicate);
        }
        set.insert(net, row);
        Ok(())
    }

    pub fn finish(self) -> PrefixIndex {
        PrefixIndex {
            table: self.table,
            keys: self.keys.into_boxed_slice(),
            sets: self.sets.into_boxed_slice(),
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
    Prefix(PrefixIndex),
}

impl Index {
    pub fn len(&self) -> usize {
        match self {
            Index::Hash(h) => h.len(),
            Index::Prefix(p) => p.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::lookup_table::prefix::parse_prefix;

    fn net(s: &str) -> IpNet {
        parse_prefix(s).unwrap()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn nested_prefixes_resolve_to_the_narrowest() {
        let mut b = PrefixIndexBuilder::new(false);
        b.insert(&[], net("10.0.0.0/8"), 1).unwrap();
        b.insert(&[], net("10.1.2.0/24"), 3).unwrap();
        b.insert(&[], net("10.1.0.0/16"), 2).unwrap();
        let index = b.finish();
        assert_eq!(index.partitions(), 1);
        assert_eq!(index.len(), 3);
        assert_eq!(
            index.get(&[], ip("10.1.2.9")),
            Some((3, net("10.1.2.0/24")))
        );
        assert_eq!(
            index.get(&[], ip("10.1.9.9")),
            Some((2, net("10.1.0.0/16")))
        );
        assert_eq!(index.get(&[], ip("10.9.9.9")), Some((1, net("10.0.0.0/8"))));
        assert_eq!(index.get(&[], ip("11.0.0.0")), None);
    }

    #[test]
    fn a_repeated_prefix_is_rejected_and_the_first_kept() {
        let mut b = PrefixIndexBuilder::new(false);
        b.insert(&[], net("10.0.0.0/8"), 1).unwrap();
        assert_eq!(
            b.insert(&[], net("10.0.0.0/8"), 2),
            Err(PrefixReject::Duplicate)
        );
        assert_eq!(
            b.finish().get(&[], ip("10.0.0.1")),
            Some((1, net("10.0.0.0/8")))
        );
    }

    #[test]
    fn partitions_isolate_identical_prefixes() {
        let mut b = PrefixIndexBuilder::new(true);
        let a = [KeyRef::String("vrf-a")];
        let c = [KeyRef::String("vrf-c")];
        b.insert(&a, net("10.0.0.0/8"), 1).unwrap();
        // the same prefix in another partition is not a duplicate
        b.insert(&c, net("10.0.0.0/8"), 2).unwrap();
        // but inside one partition it still is
        assert_eq!(
            b.insert(&a, net("10.0.0.0/8"), 3),
            Err(PrefixReject::Duplicate)
        );
        let index = b.finish();
        assert_eq!(index.partitions(), 2);
        assert_eq!(index.len(), 2);
        assert_eq!(index.get(&a, ip("10.0.0.5")), Some((1, net("10.0.0.0/8"))));
        assert_eq!(index.get(&c, ip("10.0.0.5")), Some((2, net("10.0.0.0/8"))));
        assert_eq!(index.get(&[KeyRef::String("vrf-z")], ip("10.0.0.5")), None);
        assert!(index.mem_size() > 0);
    }

    #[test]
    fn families_stay_apart() {
        let mut b = PrefixIndexBuilder::new(false);
        b.insert(&[], net("0.0.0.0/0"), 4).unwrap();
        b.insert(&[], net("2001:db8::/32"), 6).unwrap();
        let index = b.finish();
        assert_eq!(index.get(&[], ip("192.0.2.1")), Some((4, net("0.0.0.0/0"))));
        assert_eq!(
            index.get(&[], ip("2001:db8::1")),
            Some((6, net("2001:db8::/32")))
        );
        assert_eq!(index.get(&[], ip("2001:db9::1")), None);
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
