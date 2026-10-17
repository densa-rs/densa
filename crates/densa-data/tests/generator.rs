use densa_data::{MarketTable, Tick, TickGenerator, WyRand};

const SYMBOLS: u32 = 100;

fn empty_batch(n: usize) -> Vec<Tick> {
    vec![
        Tick {
            symbol_id: 0,
            bid: 0.0,
            ask: 0.0,
            last: 0.0,
            qty: 0,
            ts_ns: 0,
        };
        n
    ]
}

#[test]
fn rng_is_deterministic_per_seed() {
    let mut a = WyRand::new(42);
    let mut b = WyRand::new(42);
    let mut c = WyRand::new(43);
    let xs: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
    let ys: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
    let zs: Vec<u64> = (0..16).map(|_| c.next_u64()).collect();
    assert_eq!(xs, ys);
    assert_ne!(xs, zs);
}

#[test]
fn rng_below_stays_in_range_and_is_roughly_uniform() {
    let mut rng = WyRand::new(7);
    let mut counts = [0u32; 10];
    for _ in 0..100_000 {
        let v = rng.below(10);
        assert!(v < 10);
        counts[v as usize] += 1;
    }
    // Expected 10_000 per bucket; ±5% is many standard deviations of slack.
    assert!(
        counts.iter().all(|&c| (9_500..=10_500).contains(&c)),
        "{counts:?}"
    );
}

#[test]
fn generator_reports_symbol_count() {
    assert_eq!(TickGenerator::new(SYMBOLS, 0).symbols(), SYMBOLS);
}

#[test]
#[should_panic(expected = "at least one symbol")]
fn generator_rejects_zero_symbols() {
    let _ = TickGenerator::new(0, 0);
}

#[test]
fn generator_is_deterministic_per_seed() {
    let mut a = TickGenerator::new(SYMBOLS, 1);
    let mut b = TickGenerator::new(SYMBOLS, 1);
    let (mut xa, mut xb) = (empty_batch(256), empty_batch(256));
    a.fill(&mut xa, 5);
    b.fill(&mut xb, 5);
    assert_eq!(xa, xb);
}

#[test]
fn generated_ticks_are_well_formed() {
    let mut generator = TickGenerator::new(SYMBOLS, 3);
    let mut batch = empty_batch(1024);
    for round in 0..200u64 {
        generator.fill(&mut batch, round);
        for t in &batch {
            assert!(t.symbol_id < SYMBOLS);
            assert!(t.bid.is_finite() && t.ask.is_finite() && t.last.is_finite());
            assert!(t.bid > 0.0, "price walked to zero: {t:?}");
            assert!(t.bid < t.ask, "crossed quote: {t:?}");
            assert!(
                t.last == t.bid || t.last == t.ask,
                "trade off the quote: {t:?}"
            );
            assert!((1..=1_000).contains(&t.qty));
            assert_eq!(t.ts_ns, round);
        }
    }
}

#[test]
fn every_symbol_gets_ticks() {
    let mut generator = TickGenerator::new(SYMBOLS, 9);
    let mut batch = empty_batch(10_000);
    generator.fill(&mut batch, 0);
    let mut hit = [false; SYMBOLS as usize];
    for t in &batch {
        hit[t.symbol_id as usize] = true;
    }
    assert!(hit.iter().all(|&h| h));
}

#[test]
fn prices_move_over_time() {
    let mut generator = TickGenerator::new(1, 11);
    let mut batch = empty_batch(1_000);
    generator.fill(&mut batch, 0);
    let first = batch[0].last;
    assert!(
        batch.iter().any(|t| t.last != first),
        "random walk never moved"
    );
}

#[test]
fn generated_feed_is_fully_accepted_by_table() {
    let mut generator = TickGenerator::new(SYMBOLS, 5);
    let mut table = MarketTable::new(SYMBOLS);
    let mut batch = empty_batch(1024);
    for round in 1..=50 {
        generator.fill(&mut batch, round);
        let stats = table.apply_batch(&batch);
        assert_eq!(stats.rejected, 0);
        assert_eq!(stats.applied, 1024);
    }
}
