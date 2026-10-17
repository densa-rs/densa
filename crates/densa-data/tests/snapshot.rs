use densa_data::{MarketTable, RowSnapshot, Tick, ViewportSnapshot};

/// Table where row `i` has last price `i` so rows are easy to identify.
fn numbered_table(rows: u32) -> MarketTable {
    let mut table = MarketTable::new(rows);
    let ticks: Vec<Tick> = (0..rows)
        .map(|i| Tick {
            symbol_id: i,
            bid: f64::from(i) - 0.5,
            ask: f64::from(i) + 0.5,
            last: f64::from(i),
            qty: 1,
            ts_ns: 1,
        })
        .collect();
    assert_eq!(table.apply_batch(&ticks).rejected, 0);
    table
}

#[test]
fn copies_requested_window() {
    let table = numbered_table(10_000);
    let mut snap = ViewportSnapshot::<50>::new();
    table.snapshot_into(1_234, &mut snap);

    assert_eq!(snap.first_row, 1_234);
    assert_eq!(snap.len, 50);
    for (offset, row) in snap.visible().iter().enumerate() {
        let id = 1_234 + offset as u32;
        assert_eq!(row.symbol_id, id);
        assert_eq!(row.last, f64::from(id));
    }
}

#[test]
fn window_past_the_end_is_clamped_to_last_full_page() {
    let table = numbered_table(10_000);
    let mut snap = ViewportSnapshot::<50>::new();
    table.snapshot_into(9_990, &mut snap);
    assert_eq!(snap.first_row, 9_950);
    assert_eq!(snap.len, 50);
    assert_eq!(snap.visible().last().unwrap().symbol_id, 9_999);

    table.snapshot_into(u32::MAX, &mut snap);
    assert_eq!(snap.first_row, 9_950);
}

#[test]
fn table_smaller_than_viewport_fills_partially() {
    let table = numbered_table(3);
    let mut snap = ViewportSnapshot::<50>::new();
    table.snapshot_into(7, &mut snap);
    assert_eq!(snap.first_row, 0);
    assert_eq!(snap.len, 3);
    assert_eq!(snap.visible().len(), 3);
}

#[test]
fn empty_table_yields_empty_snapshot() {
    let table = MarketTable::new(0);
    let mut snap = ViewportSnapshot::<8>::new();
    table.snapshot_into(0, &mut snap);
    assert_eq!(snap.len, 0);
    assert!(snap.visible().is_empty());
}

#[test]
fn stale_rows_from_a_reused_buffer_are_cleared() {
    // Triple-buffer back slots hold an older snapshot; rows beyond `len` must be reset.
    let mut snap = ViewportSnapshot::<50>::new();
    numbered_table(10_000).snapshot_into(0, &mut snap);
    numbered_table(3).snapshot_into(0, &mut snap);

    assert_eq!(snap.len, 3);
    assert!(snap.rows[3..].iter().all(|r| *r == RowSnapshot::EMPTY));
}

#[test]
fn default_snapshot_is_empty() {
    let snap = ViewportSnapshot::<4>::default();
    assert_eq!(snap, ViewportSnapshot::<4>::new());
    assert!(snap.visible().is_empty());
    assert!(snap.rows.iter().all(|r| *r == RowSnapshot::EMPTY));
}

#[test]
fn snapshot_does_not_touch_sequence_metadata() {
    // `seq` and `published_at_ns` belong to the publisher, not the table.
    let table = numbered_table(10);
    let mut snap = ViewportSnapshot::<4>::new();
    snap.seq = 77;
    snap.published_at_ns = 99;
    table.snapshot_into(0, &mut snap);
    assert_eq!((snap.seq, snap.published_at_ns), (77, 99));
}
