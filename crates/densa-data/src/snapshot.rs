use crate::Direction;

/// One row of a viewport snapshot (array-of-structs: the renderer consumes whole rows).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RowSnapshot {
    /// Symbol (row) id.
    pub symbol_id: u32,
    /// `NaN` until the symbol's first quote arrives.
    pub bid: f64,
    /// Best ask; `NaN` until the first quote.
    pub ask: f64,
    /// Last traded price; `NaN` until the first trade.
    pub last: f64,
    /// Cumulative traded quantity.
    pub volume: u64,
    /// Time of the most recent tick (ns since the engine epoch); 0 if none yet.
    pub updated_at_ns: u64,
    /// Time of the last change in `last`; drives the up/down flash fade.
    pub last_change_ns: u64,
    /// Direction of the most recent change in `last`.
    pub direction: Direction,
}

impl RowSnapshot {
    /// Padding for rows beyond [`ViewportSnapshot::len`]. Never displayed.
    pub const EMPTY: Self = Self {
        symbol_id: 0,
        bid: 0.0,
        ask: 0.0,
        last: 0.0,
        volume: 0,
        updated_at_ns: 0,
        last_change_ns: 0,
        direction: Direction::Flat,
    };
}

/// Fixed-capacity copy of the visible rows, sized for a triple-buffer slot (no heap).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewportSnapshot<const N: usize> {
    /// Publisher-assigned sequence number; untouched by [`MarketTable::snapshot_into`].
    ///
    /// [`MarketTable::snapshot_into`]: crate::MarketTable::snapshot_into
    pub seq: u64,
    /// Publisher-assigned publish time (ns since engine epoch).
    pub published_at_ns: u64,
    /// Row index the snapshot actually starts at, after clamping.
    pub first_row: u32,
    /// Number of valid entries at the front of `rows`.
    pub len: u32,
    /// Row storage; only the first `len` entries are meaningful.
    pub rows: [RowSnapshot; N],
}

impl<const N: usize> ViewportSnapshot<N> {
    /// An empty snapshot (`len == 0`, `seq == 0`).
    pub const fn new() -> Self {
        Self {
            seq: 0,
            published_at_ns: 0,
            first_row: 0,
            len: 0,
            rows: [RowSnapshot::EMPTY; N],
        }
    }

    /// The valid rows, in display order.
    pub fn visible(&self) -> &[RowSnapshot] {
        &self.rows[..self.len as usize]
    }
}

impl<const N: usize> Default for ViewportSnapshot<N> {
    fn default() -> Self {
        Self::new()
    }
}
