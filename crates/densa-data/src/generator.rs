use std::num::NonZeroU32;

use crate::{Tick, WyRand};

/// Price grid: all generated prices are integer multiples of this.
const TICK_SIZE: f64 = 0.01;
/// Mid-price floor in ticks ($1.00); keeps bids positive after subtracting the spread.
const MIN_MID_TICKS: i64 = 100;
/// Initial mids are drawn from `[INITIAL_MID_MIN, INITIAL_MID_MIN + INITIAL_MID_SPAN)` ticks.
const INITIAL_MID_MIN: i64 = 1_000;
const INITIAL_MID_SPAN: u32 = 99_000;
/// Half-spread is 1..=MAX_HALF_SPREAD_TICKS ticks.
const MAX_HALF_SPREAD_TICKS: u64 = 2;
/// Trade size is 1..=MAX_QTY.
const MAX_QTY: u64 = 1_000;

/// Deterministic synthetic tick feed: a per-symbol random walk on a fixed price grid.
///
/// Symbols are picked uniformly. Each tick moves the mid by -1, 0 or +1 tick, quotes a
/// 1–2 tick half-spread around it, and prints a trade at the bid or the ask.
#[derive(Debug, Clone)]
pub struct TickGenerator {
    rng: WyRand,
    mid_ticks: Box<[i64]>,
}

impl TickGenerator {
    /// A feed over symbols `0..symbols`, reproducible for a given `seed`.
    ///
    /// `NonZeroU32` because a feed without instruments cannot pick a symbol: the type
    /// rules that state out instead of a runtime panic.
    pub fn new(symbols: NonZeroU32, seed: u64) -> Self {
        let mut rng = WyRand::new(seed);
        let mid_ticks = (0..symbols.get())
            .map(|_| INITIAL_MID_MIN + i64::from(rng.below(INITIAL_MID_SPAN)))
            .collect();
        Self { rng, mid_ticks }
    }

    /// Number of symbols the feed draws from.
    pub fn symbols(&self) -> u32 {
        // Lossless: built from a u32 count.
        self.mid_ticks.len() as u32
    }

    /// Overwrites every slot of `out` with a fresh tick stamped `now_ns`.
    pub fn fill(&mut self, out: &mut [Tick], now_ns: u64) {
        for slot in out {
            *slot = self.next_tick(now_ns);
        }
    }

    #[inline]
    fn next_tick(&mut self, ts_ns: u64) -> Tick {
        let symbol_id = self.rng.below(self.symbols());
        let bits = self.rng.next_u64();

        let step = match bits & 0b11 {
            0 => -1,
            1 => 1,
            _ => 0,
        };
        let mid = &mut self.mid_ticks[symbol_id as usize];
        *mid = (*mid + step).max(MIN_MID_TICKS);

        // Lossless: the modulo result is at most MAX_HALF_SPREAD_TICKS.
        let half_spread = 1 + ((bits >> 2) % MAX_HALF_SPREAD_TICKS) as i64;
        let bid = *mid - half_spread;
        let ask = *mid + half_spread;
        let buyer_initiated = (bits >> 4) & 1 == 1;
        let last = if buyer_initiated { ask } else { bid };

        Tick {
            symbol_id,
            bid: to_price(bid),
            ask: to_price(ask),
            last: to_price(last),
            qty: 1 + (bits >> 8) % MAX_QTY,
            ts_ns,
        }
    }
}

#[inline]
fn to_price(ticks: i64) -> f64 {
    ticks as f64 * TICK_SIZE
}
