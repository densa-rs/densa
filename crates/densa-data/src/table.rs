use crate::{Direction, RowSnapshot, Tick, ViewportSnapshot};
use std::ops::AddAssign;

/// Outcome of applying a batch of ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApplyStats {
    pub applied: u64,
    /// Ticks dropped for an unknown `symbol_id` or a non-finite price.
    pub rejected: u64,
}

impl AddAssign for ApplyStats {
    fn add_assign(&mut self, rhs: Self) {
        self.applied += rhs.applied;
        self.rejected += rhs.rejected;
    }
}

/// Market data for a dense range of symbols `0..len`, stored as structure-of-arrays.
///
/// The row index *is* the symbol id. Columns are allocated once in [`MarketTable::new`]
/// and never resized. Intended to be owned by a single ingestion thread; readers get
/// copies through [`MarketTable::snapshot_into`].
#[derive(Debug, Clone)]
pub struct MarketTable {
    bid: Box<[f64]>,
    ask: Box<[f64]>,
    last: Box<[f64]>,
    volume: Box<[u64]>,
    updated_at_ns: Box<[u64]>,
    last_change_ns: Box<[u64]>,
    direction: Box<[Direction]>,
}

fn column<T: Copy>(len: u32, value: T) -> Box<[T]> {
    vec![value; len as usize].into_boxed_slice()
}

impl MarketTable {
    /// Creates a table for symbols `0..symbols`, all without quotes (`NaN` prices).
    pub fn new(symbols: u32) -> Self {
        Self {
            bid: column(symbols, f64::NAN),
            ask: column(symbols, f64::NAN),
            last: column(symbols, f64::NAN),
            volume: column(symbols, 0),
            updated_at_ns: column(symbols, 0),
            last_change_ns: column(symbols, 0),
            direction: column(symbols, Direction::Flat),
        }
    }

    pub fn len(&self) -> u32 {
        // Lossless: `new` takes the length as a u32.
        self.last.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.last.is_empty()
    }

    /// Copy of one row, or `None` if `symbol_id` is out of range.
    pub fn row(&self, symbol_id: u32) -> Option<RowSnapshot> {
        (symbol_id < self.len()).then(|| self.row_at(symbol_id as usize))
    }

    /// Applies ticks in order. Invalid ticks are skipped and counted, never panicked on.
    pub fn apply_batch(&mut self, ticks: &[Tick]) -> ApplyStats {
        let mut stats = ApplyStats::default();
        for tick in ticks {
            if self.apply(tick) {
                stats.applied += 1;
            } else {
                stats.rejected += 1;
            }
        }
        stats
    }

    /// Copies up to `N` rows starting at `first_row` into `out`.
    ///
    /// `first_row` is clamped so the window ends at the last row (a full page when the
    /// table has at least `N` rows). Rows past `out.len` are reset to
    /// [`RowSnapshot::EMPTY`], since `out` may be a reused, stale buffer. `out.seq` and
    /// `out.published_at_ns` are left to the publisher.
    pub fn snapshot_into<const N: usize>(&self, first_row: u32, out: &mut ViewportSnapshot<N>) {
        let total = self.last.len();
        let count = N.min(total);
        let first = (first_row as usize).min(total - count);

        for (offset, slot) in out.rows.iter_mut().enumerate() {
            *slot = if offset < count {
                self.row_at(first + offset)
            } else {
                RowSnapshot::EMPTY
            };
        }
        // Lossless: both are bounded by `total`, which fits in u32.
        out.first_row = first as u32;
        out.len = count as u32;
    }

    fn apply(&mut self, tick: &Tick) -> bool {
        let i = tick.symbol_id as usize;
        if i >= self.last.len() {
            return false;
        }
        if !(tick.bid.is_finite() && tick.ask.is_finite() && tick.last.is_finite()) {
            return false;
        }

        // A `NaN` previous price (no trade yet) compares false both ways, so the first
        // trade is `Flat` and does not trigger a spurious flash.
        let previous = self.last[i];
        if tick.last > previous {
            self.direction[i] = Direction::Up;
            self.last_change_ns[i] = tick.ts_ns;
        } else if tick.last < previous {
            self.direction[i] = Direction::Down;
            self.last_change_ns[i] = tick.ts_ns;
        }

        self.bid[i] = tick.bid;
        self.ask[i] = tick.ask;
        self.last[i] = tick.last;
        self.volume[i] = self.volume[i].saturating_add(tick.qty);
        self.updated_at_ns[i] = tick.ts_ns;
        true
    }

    fn row_at(&self, i: usize) -> RowSnapshot {
        RowSnapshot {
            // Lossless: `i < len`, which fits in u32.
            symbol_id: i as u32,
            bid: self.bid[i],
            ask: self.ask[i],
            last: self.last[i],
            volume: self.volume[i],
            updated_at_ns: self.updated_at_ns[i],
            last_change_ns: self.last_change_ns[i],
            direction: self.direction[i],
        }
    }
}
