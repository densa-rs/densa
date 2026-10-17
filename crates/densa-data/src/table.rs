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

/// One symbol's state: 56 bytes, so an update touches one or two cache lines.
#[derive(Debug, Clone, Copy)]
struct Row {
    bid: f64,
    ask: f64,
    last: f64,
    volume: u64,
    updated_at_ns: u64,
    last_change_ns: u64,
    direction: Direction,
}

impl Row {
    /// No quote yet: `NaN` prices.
    const EMPTY: Self = Self {
        bid: f64::NAN,
        ask: f64::NAN,
        last: f64::NAN,
        volume: 0,
        updated_at_ns: 0,
        last_change_ns: 0,
        direction: Direction::Flat,
    };
}

/// Market data for a dense range of symbols `0..len`, stored as one flat array of rows.
///
/// The row index *is* the symbol id. Rows are allocated once in [`MarketTable::new`] and
/// never resized. Intended to be owned by a single ingestion thread; readers get copies
/// through [`MarketTable::snapshot_into`].
///
/// Array-of-structs, not structure-of-arrays: every tick updates one random row, which
/// costs one or two cache lines here but one line *per column* in SoA. Measured with
/// `benches/ingest.rs` at 1M symbols (beyond L3), applying ticks is ~1.7x faster than SoA;
/// within cache both layouts are equal. Revisit if column scans (aggregates, SIMD) become
/// the hot path.
#[derive(Debug, Clone)]
pub struct MarketTable {
    rows: Box<[Row]>,
}

impl MarketTable {
    /// Creates a table for symbols `0..symbols`, all without quotes (`NaN` prices).
    pub fn new(symbols: u32) -> Self {
        Self {
            rows: vec![Row::EMPTY; symbols as usize].into_boxed_slice(),
        }
    }

    pub fn len(&self) -> u32 {
        // Lossless: `new` takes the length as a u32.
        self.rows.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Copy of one row, or `None` if `symbol_id` is out of range.
    pub fn row(&self, symbol_id: u32) -> Option<RowSnapshot> {
        self.rows
            .get(symbol_id as usize)
            .map(|row| snapshot_of(symbol_id, row))
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
        let total = self.rows.len();
        let count = N.min(total);
        let first = (first_row as usize).min(total - count);
        let window = &self.rows[first..first + count];

        for (offset, slot) in out.rows.iter_mut().enumerate() {
            *slot = match window.get(offset) {
                // Lossless: bounded by `total`, which fits in u32.
                Some(row) => snapshot_of((first + offset) as u32, row),
                None => RowSnapshot::EMPTY,
            };
        }
        // Lossless: both are bounded by `total`, which fits in u32.
        out.first_row = first as u32;
        out.len = count as u32;
    }

    fn apply(&mut self, tick: &Tick) -> bool {
        let Some(row) = self.rows.get_mut(tick.symbol_id as usize) else {
            return false;
        };
        if !(tick.bid.is_finite() && tick.ask.is_finite() && tick.last.is_finite()) {
            return false;
        }

        // A `NaN` previous price (no trade yet) compares false both ways, so the first
        // trade is `Flat` and does not trigger a spurious flash.
        if tick.last > row.last {
            row.direction = Direction::Up;
            row.last_change_ns = tick.ts_ns;
        } else if tick.last < row.last {
            row.direction = Direction::Down;
            row.last_change_ns = tick.ts_ns;
        }

        row.bid = tick.bid;
        row.ask = tick.ask;
        row.last = tick.last;
        row.volume = row.volume.saturating_add(tick.qty);
        row.updated_at_ns = tick.ts_ns;
        true
    }
}

fn snapshot_of(symbol_id: u32, row: &Row) -> RowSnapshot {
    RowSnapshot {
        symbol_id,
        bid: row.bid,
        ask: row.ask,
        last: row.last,
        volume: row.volume,
        updated_at_ns: row.updated_at_ns,
        last_change_ns: row.last_change_ns,
        direction: row.direction,
    }
}
