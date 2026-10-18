//! A sorted sequence of rows kept in blocks, so inserting or removing one row moves at most
//! one block's worth of entries (like Python's `sortedcontainers`), not the whole view. A
//! Fenwick tree over the block lengths finds the row at an index in `O(log blocks)`.

use std::cmp::Ordering;

use crate::RowId;

/// Entries per block before it splits in two.
const MAX_BLOCK: usize = 1_024;
/// Entries per block when building from scratch.
const BUILD_BLOCK: usize = MAX_BLOCK / 2;
/// A block shrinking below this merges with a neighbor, so deletes cannot leave many tiny
/// blocks behind.
const MIN_BLOCK: usize = MAX_BLOCK / 8;

/// A row in a view: its id and a 64-bit prefix of its sort key, kept next to the id so a
/// search compares contiguous memory and reads the row's values only when prefixes tie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) key: u64,
    pub(crate) id: RowId,
}

/// Entries in the order of a comparator the caller passes to every call. The comparator must
/// be a total order consistent across calls (the caller breaks ties by row id).
#[derive(Debug, Clone, Default)]
pub(crate) struct SortedRows {
    blocks: Vec<Vec<Entry>>,
    len: usize,
    /// Fenwick tree (1-based) of block lengths.
    tree: Vec<usize>,
}

impl SortedRows {
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// The id at `index`, counting from the first.
    pub(crate) fn get(&self, index: usize) -> Option<RowId> {
        if index >= self.len {
            return None;
        }
        // Descend the Fenwick tree to the last block whose preceding lengths sum to at most
        // `index`.
        let (mut block, mut rest) = (0, index);
        let mut step = self.blocks.len().checked_ilog2().map_or(0, |log| 1 << log);
        while step > 0 {
            let next = block + step;
            if next <= self.blocks.len() && self.tree[next] <= rest {
                block = next;
                rest -= self.tree[next];
            }
            step >>= 1;
        }
        self.blocks.get(block)?.get(rest).map(|entry| entry.id)
    }

    /// The first id. Blocks are never empty, so it heads the first block.
    pub(crate) fn first(&self) -> Option<RowId> {
        self.blocks.first()?.first().map(|entry| entry.id)
    }

    /// The last id.
    pub(crate) fn last(&self) -> Option<RowId> {
        self.blocks.last()?.last().map(|entry| entry.id)
    }

    /// Replaces the contents with `entries`, which must already be in order.
    pub(crate) fn rebuild(&mut self, entries: &[Entry]) {
        self.blocks = entries.chunks(BUILD_BLOCK).map(<[Entry]>::to_vec).collect();
        self.len = entries.len();
        self.rebuild_tree();
    }

    /// Inserts `entry`; `order(other)` says where `other` sorts relative to it.
    pub(crate) fn insert(&mut self, entry: Entry, order: impl Fn(Entry) -> Ordering) {
        if self.blocks.is_empty() {
            // Grows as needed: groups keep one of these per group, most of them small.
            self.blocks.push(Vec::new());
            self.rebuild_tree();
        }
        // The first block whose last entry sorts after `entry`; past the end, the last block.
        let block = self
            .blocks
            .partition_point(|b| b.last().is_some_and(|&last| order(last).is_lt()))
            .min(self.blocks.len() - 1);
        let entries = &mut self.blocks[block];
        let at = entries.partition_point(|&other| order(other).is_lt());
        entries.insert(at, entry);
        self.len += 1;
        if entries.len() > MAX_BLOCK {
            let upper = entries.split_off(entries.len() / 2);
            self.blocks.insert(block + 1, upper);
            self.rebuild_tree();
        } else {
            self.add(block, 1);
        }
    }

    /// Removes the entry of row `id`, found with `order` as for [`insert`](Self::insert);
    /// `false` if absent.
    pub(crate) fn remove(&mut self, id: RowId, order: impl Fn(Entry) -> Ordering) -> bool {
        let block = self
            .blocks
            .partition_point(|b| b.last().is_some_and(|&last| order(last).is_lt()));
        let Some(entries) = self.blocks.get_mut(block) else {
            return false;
        };
        let at = entries.partition_point(|&other| order(other).is_lt());
        if entries.get(at).map(|e| e.id) != Some(id) {
            return false;
        }
        entries.remove(at);
        self.len -= 1;
        if entries.len() < MIN_BLOCK && self.merge_around(block) {
            self.rebuild_tree();
        } else {
            self.subtract(block, 1);
        }
        true
    }

    /// The index of row `id`, found with `order` as for [`insert`](Self::insert); `None` if
    /// absent.
    pub(crate) fn position(&self, id: RowId, order: impl Fn(Entry) -> Ordering) -> Option<usize> {
        let block = self
            .blocks
            .partition_point(|b| b.last().is_some_and(|&last| order(last).is_lt()));
        let entries = self.blocks.get(block)?;
        let at = entries.partition_point(|&other| order(other).is_lt());
        (entries.get(at)?.id == id).then(|| self.before(block) + at)
    }

    /// Entries in the blocks before `block`.
    fn before(&self, block: usize) -> usize {
        let (mut i, mut sum) = (block, 0);
        while i > 0 {
            sum += self.tree[i];
            i &= i - 1;
        }
        sum
    }

    /// Drops block `block` if empty, or merges it into a neighbor if both fit in one block.
    /// Returns whether the blocks changed.
    fn merge_around(&mut self, block: usize) -> bool {
        if self.blocks[block].is_empty() {
            self.blocks.remove(block);
            return true;
        }
        let fits = |a: usize, b: usize| a + b <= MAX_BLOCK;
        let len = self.blocks[block].len();
        if block + 1 < self.blocks.len() && fits(len, self.blocks[block + 1].len()) {
            let next = self.blocks.remove(block + 1);
            self.blocks[block].extend(next);
            return true;
        }
        if block > 0 && fits(self.blocks[block - 1].len(), len) {
            let this = self.blocks.remove(block);
            self.blocks[block - 1].extend(this);
            return true;
        }
        false
    }

    fn rebuild_tree(&mut self) {
        self.tree.clear();
        self.tree.resize(self.blocks.len() + 1, 0);
        for (i, block) in self.blocks.iter().enumerate() {
            self.tree[i + 1] += block.len();
            let parent = (i + 1) + ((i + 1) & (i + 1).wrapping_neg());
            if parent <= self.blocks.len() {
                let carried = self.tree[i + 1];
                self.tree[parent] += carried;
            }
        }
    }

    fn add(&mut self, block: usize, count: usize) {
        let mut i = block + 1;
        while i < self.tree.len() {
            self.tree[i] += count;
            i += i & i.wrapping_neg();
        }
    }

    fn subtract(&mut self, block: usize, count: usize) {
        let mut i = block + 1;
        while i < self.tree.len() {
            self.tree[i] -= count;
            i += i & i.wrapping_neg();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Orders ids by a scrambled key, so sorted order differs from id order.
    fn key(id: RowId) -> u64 {
        u64::from(id).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40
    }

    fn entry(id: RowId) -> Entry {
        Entry { key: key(id), id }
    }

    fn order(id: RowId) -> impl Fn(Entry) -> Ordering {
        move |other| (other.key, other.id).cmp(&(key(id), id))
    }

    #[test]
    fn many_blocks_stay_in_order_through_inserts_and_removes() {
        let mut rows = SortedRows::default();
        let mut model = BTreeSet::new();
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            // Lossless: below 5,000.
            let id = (state % 5_000) as RowId;
            if model.contains(&(key(id), id)) {
                assert!(rows.remove(id, order(id)));
                model.remove(&(key(id), id));
            } else {
                rows.insert(entry(id), order(id));
                model.insert((key(id), id));
            }
        }
        assert!(rows.blocks.len() > 1, "the test must span several blocks");
        let ids: Vec<RowId> = (0..rows.len()).filter_map(|i| rows.get(i)).collect();
        let expected: Vec<RowId> = model.iter().map(|&(_, id)| id).collect();
        assert_eq!(ids, expected);
        for (index, &id) in ids.iter().enumerate() {
            assert_eq!(rows.position(id, order(id)), Some(index), "id {id}");
        }
        let absent = (0..5_000)
            .find(|id| !model.contains(&(key(*id), *id)))
            .unwrap();
        assert_eq!(rows.position(absent, order(absent)), None);
        assert_eq!(rows.get(rows.len()), None);
        assert_eq!(rows.first(), expected.first().copied());
        assert_eq!(rows.last(), expected.last().copied());
    }

    #[test]
    fn deletes_merge_small_blocks_and_indexing_stays_exact() {
        let mut rows = SortedRows::default();
        for id in 0..20_000 {
            rows.insert(entry(id), order(id));
        }
        let before = rows.blocks.len();
        let mut model: BTreeSet<(u64, RowId)> = (0..20_000).map(|id| (key(id), id)).collect();
        for id in (0..20_000).filter(|id| id % 20 != 0) {
            assert!(rows.remove(id, order(id)));
            model.remove(&(key(id), id));
        }
        assert!(
            rows.blocks.len() * 4 < before,
            "{} blocks left of {before}",
            rows.blocks.len()
        );
        assert!(rows.blocks.iter().all(|b| !b.is_empty()));
        let expected: Vec<RowId> = model.iter().map(|&(_, id)| id).collect();
        let ids: Vec<RowId> = (0..rows.len()).filter_map(|i| rows.get(i)).collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn removing_an_absent_id_changes_nothing() {
        let mut rows = SortedRows::default();
        for id in [1, 3, 5] {
            rows.insert(entry(id), order(id));
        }
        assert!(!rows.remove(4, order(4)));
        assert_eq!(rows.len(), 3);
        assert!(!SortedRows::default().remove(1, order(1)));
        assert_eq!(SortedRows::default().first(), None);
        assert_eq!(SortedRows::default().last(), None);
    }
}
