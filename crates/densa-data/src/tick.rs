/// One market data update: a quote refresh plus the trade that caused it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tick {
    /// Row of the symbol in the `MarketTable`.
    pub symbol_id: u32,
    /// Best bid price.
    pub bid: f64,
    /// Best ask price.
    pub ask: f64,
    /// Last traded price.
    pub last: f64,
    /// Traded quantity, added to the symbol's cumulative volume.
    pub qty: u64,
    /// Nanoseconds since the engine epoch.
    pub ts_ns: u64,
}

/// Direction of the most recent change in last-traded price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i8)]
pub enum Direction {
    /// The last price went down.
    Down = -1,
    /// No price change seen yet.
    #[default]
    Flat = 0,
    /// The last price went up.
    Up = 1,
}
