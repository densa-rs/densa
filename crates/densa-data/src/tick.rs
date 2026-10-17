/// One market data update: a quote refresh plus the trade that caused it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tick {
    pub symbol_id: u32,
    pub bid: f64,
    pub ask: f64,
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
    Down = -1,
    #[default]
    Flat = 0,
    Up = 1,
}
