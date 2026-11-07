//! Summaries of series too long to visit each frame: runs of their items, and runs of runs,
//! each holding what drawing and scaling need of its items, so a frame visits a few nodes per
//! pixel column instead of every item that shows.

use std::ops::Range;

mod bars;
mod candles;
mod line;

pub use bars::{paint_bars_summarized, BarSummary};
pub use candles::{paint_candles_summarized, CandleSummary};
pub use line::{paint_line_summarized, LineSummary};

/// Items in a node of the lowest level, and nodes of one level in a node of the next.
const LEAF: usize = 32;
const FAN: usize = 8;

/// No such item: a node holding none that counts.
const NONE: usize = usize::MAX;

/// What a run of items holds.
trait Run: Copy {
    /// A run of no items.
    const EMPTY: Self;

    /// This run and the one right after it, as one.
    fn then(self, next: Self) -> Self;
}

/// A series' items summarized: level 0 holds runs of [`LEAF`] items, each level above runs of
/// [`FAN`] nodes of the one below, and the top one node. The series must not change but by
/// items added at the end.
#[derive(Debug, Clone)]
struct Levels<N> {
    /// The items summarized.
    len: usize,
    /// The `x` of the first and last of them, to tell them from others of the same count.
    ends: (f64, f64),
    levels: Vec<Vec<N>>,
}

impl<N> Default for Levels<N> {
    fn default() -> Self {
        Self {
            len: 0,
            ends: (0.0, 0.0),
            levels: Vec::new(),
        }
    }
}

/// What a walk over the items of a range does with them.
trait Visit<N> {
    /// Takes node `node`, which holds the items `run`, all within the range, as one if it can
    /// stand for them (`true`), or leaves them to be visited one at a time (`false`).
    fn node(&mut self, node: N, run: Range<usize>) -> bool;

    /// Item `index` itself.
    fn item(&mut self, index: usize);
}

impl<N: Run> Levels<N> {
    /// Takes in the items past those it summarizes (`len` of them, item `i` at `x(i)`), the
    /// earlier ones taken as they were; `leaf` summarizes a run of them. Fewer items than it
    /// summarizes, or a first item or a last summarized item at another `x` (a window that
    /// slid), are summarized anew.
    fn update(&mut self, len: usize, x: impl Fn(usize) -> f64, leaf: impl Fn(Range<usize>) -> N) {
        if len < self.len || !self.same_ends(&x, self.len) {
            self.clear();
        }
        if len == self.len {
            return;
        }
        if self.levels.is_empty() {
            self.levels.push(Vec::new());
        }
        // The run holding the first new item, and every run after it, at each level.
        let mut from = self.len / LEAF;
        let leaves = &mut self.levels[0];
        leaves.truncate(from);
        for start in (from * LEAF..len).step_by(LEAF) {
            leaves.push(leaf(start..(start + LEAF).min(len)));
        }
        let mut level = 0;
        while self.levels[level].len() > 1 {
            from /= FAN;
            if self.levels.len() == level + 1 {
                self.levels.push(Vec::new());
            }
            let (below, above) = self.levels.split_at_mut(level + 1);
            let (below, above) = (&below[level], &mut above[0]);
            above.truncate(from);
            for start in (from * FAN..below.len()).step_by(FAN) {
                let run = &below[start..(start + FAN).min(below.len())];
                above.push(run.iter().fold(N::EMPTY, |all, &node| all.then(node)));
            }
            level += 1;
        }
        self.levels.truncate(level + 1);
        self.len = len;
        self.ends = (x(0), x(len - 1));
    }

    /// Summarizes `len` items anew, keeping the memory it holds.
    fn rebuild(&mut self, len: usize, x: impl Fn(usize) -> f64, leaf: impl Fn(Range<usize>) -> N) {
        self.clear();
        self.update(len, x, leaf);
    }

    /// Summarizes no items, keeping the memory it holds.
    fn clear(&mut self) {
        self.len = 0;
        for level in &mut self.levels {
            level.clear();
        }
    }

    /// Whether the first item and the one at `count - 1` are at the `x` of those it summarizes
    /// (none summarized: yes).
    fn same_ends(&self, x: impl Fn(usize) -> f64, count: usize) -> bool {
        count == 0 || (x(0) == self.ends.0 && x(count - 1) == self.ends.1)
    }

    /// Whether it summarizes `len` items at `x`, as far as their count and the `x` of their
    /// ends tell.
    fn covers(&self, len: usize, x: impl Fn(usize) -> f64) -> bool {
        self.len == len && self.same_ends(x, len)
    }

    /// What the items of `range` (the part of it within those summarized) hold, as one run:
    /// whole nodes where they lie within it, `leaf` of the items at its ends.
    fn fold(&self, range: Range<usize>, leaf: impl Fn(Range<usize>) -> N) -> N {
        let range = range.start.min(self.len)..range.end.min(self.len);
        let mut found = N::EMPTY;
        self.each_top(|level, index| self.fold_node(level, index, &range, &leaf, &mut found));
        found
    }

    fn fold_node(
        &self,
        level: usize,
        index: usize,
        range: &Range<usize>,
        leaf: &impl Fn(Range<usize>) -> N,
        found: &mut N,
    ) {
        let (start, end) = self.bounds(level, index);
        if end <= range.start || start >= range.end {
            return;
        }
        if range.start <= start && end <= range.end {
            *found = found.then(self.levels[level][index]);
        } else if level == 0 {
            *found = found.then(leaf(start.max(range.start)..end.min(range.end)));
        } else {
            for child in self.children(level, index) {
                self.fold_node(level - 1, child, range, leaf, found);
            }
        }
    }

    /// Walks the items of `range` in order: each node wholly within it offered to `visit` as
    /// one, and the items of those it does not take (and of nodes partly within it) one at a
    /// time.
    fn walk(&self, visit: &mut impl Visit<N>, range: &Range<usize>) {
        self.each_top(|level, index| self.walk_node(visit, level, index, range));
    }

    fn walk_node(
        &self,
        visit: &mut impl Visit<N>,
        level: usize,
        index: usize,
        range: &Range<usize>,
    ) {
        let (start, end) = self.bounds(level, index);
        if end <= range.start || start >= range.end {
            return;
        }
        let within = range.start <= start && end <= range.end;
        if within && visit.node(self.levels[level][index], start..end) {
            return;
        }
        if level == 0 {
            for at in start.max(range.start)..end.min(range.end) {
                visit.item(at);
            }
        } else {
            for child in self.children(level, index) {
                self.walk_node(visit, level - 1, child, range);
            }
        }
    }

    /// The items of node `index` of `level`.
    fn bounds(&self, level: usize, index: usize) -> (usize, usize) {
        // Saturating: a node larger than any count of items holds them all.
        let levels = u32::try_from(level).unwrap_or(u32::MAX);
        let size = LEAF.saturating_mul(FAN.saturating_pow(levels));
        let start = index.saturating_mul(size);
        (start, start.saturating_add(size).min(self.len))
    }

    /// The nodes below node `index` of `level` (above level 0).
    fn children(&self, level: usize, index: usize) -> Range<usize> {
        let below = self.levels[level - 1].len();
        (index * FAN).min(below)..((index + 1) * FAN).min(below)
    }

    /// Calls `visit(level, index)` for each node of the top level.
    fn each_top(&self, mut visit: impl FnMut(usize, usize)) {
        if let Some(top) = self.levels.len().checked_sub(1) {
            for index in 0..self.levels[top].len() {
                visit(top, index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many items a run holds.
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Count(usize);

    impl Run for Count {
        const EMPTY: Self = Self(0);

        fn then(self, next: Self) -> Self {
            Self(self.0 + next.0)
        }
    }

    fn counted(len: usize) -> Levels<Count> {
        let mut levels = Levels::default();
        levels.update(len, |i| i as f64, |run| Count(run.len()));
        levels
    }

    #[test]
    fn levels_go_up_until_one_node_holds_everything() {
        let levels = counted(LEAF * FAN * FAN + 1);
        let sizes: Vec<usize> = levels.levels.iter().map(Vec::len).collect();
        assert_eq!(sizes, [FAN * FAN + 1, FAN + 1, 2, 1]);
        assert_eq!(levels.levels[3][0], Count(LEAF * FAN * FAN + 1));
        assert_eq!(counted(5).levels.len(), 1);
        assert_eq!(counted(0).levels.len(), 0);
    }

    #[test]
    fn a_fold_counts_the_items_of_its_range_only() {
        let levels = counted(10_000);
        assert_eq!(levels.fold(37..9_001, |run| Count(run.len())), Count(8_964));
        assert_eq!(
            levels.fold(9_990..20_000, |run| Count(run.len())),
            Count(10)
        );
        assert_eq!(levels.fold(5..5, |run| Count(run.len())), Count(0));
    }
}
