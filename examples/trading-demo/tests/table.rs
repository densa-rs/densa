//! Market table: tick application, direction tracking, rejection of invalid ticks.

use trading_demo::data::{ApplyStats, Direction, MarketTable, Tick};

fn tick(symbol_id: u32, bid: f64, ask: f64, last: f64, qty: u64, ts_ns: u64) -> Tick {
    Tick {
        symbol_id,
        bid,
        ask,
        last,
        qty,
        ts_ns,
    }
}

#[test]
fn new_table_has_no_quotes() {
    let table = MarketTable::new(3);
    assert_eq!(table.len(), 3);
    let row = table.row(2).expect("row in range");
    assert_eq!(row.symbol_id, 2);
    assert!(row.bid.is_nan() && row.ask.is_nan() && row.last.is_nan());
    assert_eq!(row.volume, 0);
    assert_eq!(row.direction, Direction::Flat);
    assert!(table.row(3).is_none());
    assert!(!table.is_empty());
    assert!(MarketTable::new(0).is_empty());
}

#[test]
fn apply_overwrites_quote_and_accumulates_volume() {
    let mut table = MarketTable::new(4);
    let stats = table.apply_batch(&[
        tick(1, 10.0, 10.2, 10.1, 5, 100),
        tick(1, 10.1, 10.3, 10.2, 7, 200),
    ]);
    assert_eq!(
        stats,
        ApplyStats {
            applied: 2,
            rejected: 0
        }
    );
    let row = table.row(1).unwrap();
    assert_eq!((row.bid, row.ask, row.last), (10.1, 10.3, 10.2));
    assert_eq!(row.volume, 12);
    assert_eq!(row.updated_at_ns, 200);
    // Other rows untouched.
    assert!(table.row(0).unwrap().last.is_nan());
}

#[test]
fn first_trade_is_flat_then_direction_follows_last_price() {
    let mut table = MarketTable::new(1);
    table.apply_batch(&[tick(0, 9.9, 10.1, 10.0, 1, 10)]);
    assert_eq!(table.row(0).unwrap().direction, Direction::Flat);

    table.apply_batch(&[tick(0, 10.0, 10.2, 10.1, 1, 20)]);
    let row = table.row(0).unwrap();
    assert_eq!(row.direction, Direction::Up);
    assert_eq!(row.last_change_ns, 20);

    table.apply_batch(&[tick(0, 9.8, 10.0, 9.9, 1, 30)]);
    let row = table.row(0).unwrap();
    assert_eq!(row.direction, Direction::Down);
    assert_eq!(row.last_change_ns, 30);
}

#[test]
fn unchanged_last_price_keeps_previous_direction_and_change_time() {
    let mut table = MarketTable::new(1);
    table.apply_batch(&[
        tick(0, 9.9, 10.1, 10.0, 1, 10),
        tick(0, 10.0, 10.2, 10.1, 1, 20),
        // Quote moves, trade price does not: the flash must not restart.
        tick(0, 10.05, 10.15, 10.1, 1, 30),
    ]);
    let row = table.row(0).unwrap();
    assert_eq!(row.direction, Direction::Up);
    assert_eq!(row.last_change_ns, 20);
    assert_eq!(row.updated_at_ns, 30);
}

#[test]
fn rejects_out_of_range_symbol_and_leaves_table_untouched() {
    let mut table = MarketTable::new(2);
    let stats = table.apply_batch(&[
        tick(2, 1.0, 1.1, 1.05, 1, 1),
        tick(u32::MAX, 1.0, 1.1, 1.05, 1, 1),
    ]);
    assert_eq!(stats.rejected, 2);
    assert_eq!(stats.applied, 0);
    for id in 0..2 {
        assert_eq!(table.row(id).unwrap().volume, 0);
    }
}

#[test]
fn rejects_non_finite_prices() {
    let mut table = MarketTable::new(1);
    let stats = table.apply_batch(&[
        tick(0, f64::NAN, 1.1, 1.05, 1, 1),
        tick(0, 1.0, f64::INFINITY, 1.05, 1, 1),
        tick(0, 1.0, 1.1, f64::NEG_INFINITY, 1, 1),
    ]);
    assert_eq!(stats.rejected, 3);
    assert!(table.row(0).unwrap().last.is_nan());
}

#[test]
fn negative_prices_are_valid() {
    // Calendar spreads and some energy futures legitimately trade below zero.
    let mut table = MarketTable::new(1);
    let stats = table.apply_batch(&[tick(0, -1.2, -1.0, -1.1, 3, 1)]);
    assert_eq!(stats.applied, 1);
    assert_eq!(table.row(0).unwrap().last, -1.1);
}

#[test]
fn volume_saturates_instead_of_wrapping() {
    let mut table = MarketTable::new(1);
    table.apply_batch(&[
        tick(0, 1.0, 1.1, 1.05, u64::MAX, 1),
        tick(0, 1.0, 1.1, 1.05, 5, 2),
    ]);
    assert_eq!(table.row(0).unwrap().volume, u64::MAX);
}

#[test]
fn apply_stats_accumulate() {
    let mut total = ApplyStats::default();
    total += ApplyStats {
        applied: 3,
        rejected: 1,
    };
    total += ApplyStats {
        applied: 2,
        rejected: 0,
    };
    assert_eq!(total.applied, 5);
    assert_eq!(total.rejected, 1);
}
