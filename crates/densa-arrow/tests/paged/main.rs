//! The paged model: rows fetched a page at a time from a source, in the order a click asks
//! for, only the pages shown and one on each side, on a thread of the model's own.

mod failures;
mod support;

use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use arrow_array::{ArrayRef, RecordBatch, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use densa_arrow::{PageError, PageOptions, PageQuery, PageSource, PagedModel, SortKey, TableError};
use densa_grid::{GridModel, SortOrder};
use support::*;

#[test]
fn rows_come_a_page_at_a_time_and_only_the_pages_shown() {
    let (source, calls) = Numbers::new(1_000_000);
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    assert_eq!(model.rows(), 1_000_000);
    assert_eq!(model.pages_held(), 0, "nothing shown, nothing fetched");

    model.showing(0..40);
    until(&mut model, |m| m.is_fetched(0) && m.is_fetched(100));
    assert_eq!(cell(&model, 39, 0), ("39".to_owned(), true));
    assert_eq!(cell(&model, 39, 1).0, "row 39");

    // A jump to the middle: its page and one on each side, none between, in one call.
    model.showing(500_000..500_040);
    until(&mut model, |m| {
        m.is_fetched(499_999) && m.is_fetched(500_000) && m.is_fetched(500_100)
    });
    assert_eq!(cell(&model, 500_039, 0).0, "500039");
    assert_eq!(pages_asked(&calls, 100), [0, 1, 4_999, 5_000, 5_001]);
    assert_eq!(
        page_calls(&calls).len(),
        2,
        "pages side by side come in one call"
    );
    assert_eq!(model.pages_held(), 5);
}

#[test]
fn the_last_rows_ask_for_no_page_past_them() {
    let (source, calls) = Numbers::new(250);
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(210..250);
    until(&mut model, |m| m.is_fetched(249) && m.is_fetched(199));
    assert_eq!(cell(&model, 249, 0).0, "249");
    assert_eq!(pages_asked(&calls, 100), [1, 2]);
}

#[test]
fn cells_are_empty_and_dim_until_their_page_arrives() {
    let (source, _) = Numbers::new(1_000);
    let (source, go) = source.gated();
    let mut model = model(source, 100);
    go.send(()).unwrap(); // the count
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    model.prepare();
    assert_eq!(cell(&model, 5, 0), (String::new(), false));
    go.send(()).unwrap();
    until(&mut model, |m| m.is_fetched(5));
    assert_eq!(cell(&model, 5, 0), ("5".to_owned(), true));
}

#[test]
fn a_click_sorts_at_the_source_without_counting_again() {
    let (source, calls) = Numbers::new(1_000_000);
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| m.is_fetched(0));
    assert_eq!(cell(&model, 0, 0).0, "0");

    assert!(model.sort(0, SortOrder::Descending));
    assert_eq!(model.sort_state(), Some((0, SortOrder::Descending)));
    assert!(!model.is_fetched(0), "the old order's pages are gone");
    assert_eq!(cell(&model, 0, 0).0, "", "and not shown");
    assert_eq!(model.rows(), 1_000_000, "a new order, as many rows");
    model.showing(0..10);
    until(&mut model, |m| m.is_fetched(0));
    assert_eq!(cell(&model, 0, 0).0, "999999");

    assert_eq!(counts(&calls), 1, "sorting changes no count");
    let descending = Some(("n".to_owned(), SortOrder::Descending));
    assert!(calls.lock().unwrap().contains(&Call::Page {
        offset: 0,
        limit: 200,
        sort: descending
    }));
}

#[test]
fn only_a_column_named_once_and_the_source_sorts_by_sorts() {
    let (mut source, _) = Numbers::new(10);
    source.unsortable = vec!["third"];
    let twice = Arc::new(Schema::new(vec![
        Field::new("n", DataType::Int64, false),
        Field::new("n", DataType::Utf8, false),
        Field::new("third", DataType::Float64, false),
    ]));
    let mut model = PagedModel::new(twice, source, || {}).unwrap();
    assert!(model.sort(0, SortOrder::Ascending));
    assert!(
        !model.sort(1, SortOrder::Ascending),
        "the source sorts by name"
    );
    assert!(!model.sort(2, SortOrder::Ascending), "the source cannot");
    assert!(!model.sort(3, SortOrder::Ascending), "no such column");
    assert_eq!(model.sort_state(), Some((0, SortOrder::Ascending)));
}

#[test]
fn pages_far_from_those_shown_are_dropped() {
    let (source, _) = Numbers::new(1_000_000);
    let options = PageOptions::default().page_rows(10).pages_kept(3);
    let mut model = PagedModel::with_options(schema(), source, || {}, options).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| m.is_fetched(0) && m.is_fetched(10));
    model.showing(1_000..1_010);
    until(&mut model, |m| {
        m.is_fetched(990) && m.is_fetched(1_000) && m.is_fetched(1_010)
    });
    assert_eq!(model.pages_held(), 3);
    assert!(!model.is_fetched(0) && !model.is_fetched(10));
}

#[test]
fn pages_shown_are_kept_even_past_the_most_kept() {
    let (source, _) = Numbers::new(1_000_000);
    let options = PageOptions::default().page_rows(10).pages_kept(1);
    let mut model = PagedModel::with_options(schema(), source, || {}, options).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    // Rows 20..60: pages 2 to 5, and 1 and 6 beside them.
    model.showing(20..60);
    until(&mut model, |m| {
        (10..70).step_by(10).all(|row| m.is_fetched(row))
    });
    assert_eq!(model.pages_held(), 6);
}

#[test]
fn pages_queued_and_no_longer_wanted_are_skipped_then_asked_again() {
    let (source, calls) = Numbers::new(1_000_000);
    let (source, go) = source.gated();
    let mut model = model(source, 10);
    go.send(()).unwrap(); // the count
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10); // pages 0 and 1: one call, held at the gate
    model.showing(5_000..5_010); // pages 499 to 501, queued behind it
    model.showing(10_000..10_010); // pages 999 to 1001: what is wanted now
    for _ in 0..6 {
        go.send(()).unwrap();
    }
    until(&mut model, |m| {
        m.is_fetched(9_990) && m.is_fetched(10_000) && m.is_fetched(10_010)
    });
    std::thread::sleep(Duration::from_millis(50));
    model.prepare();
    let asked = pages_asked(&calls, 10);
    assert!(
        !asked.iter().any(|page| (499..=501).contains(page)),
        "pages no longer wanted are skipped: {asked:?}"
    );
    // Back there: what was skipped is asked again.
    model.showing(5_000..5_010);
    until(&mut model, |m| m.is_fetched(5_000));
}

#[test]
fn a_page_in_several_batches_is_joined() {
    let (mut source, _) = Numbers::new(1_000);
    source.halves = true;
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..40);
    until(&mut model, |m| m.is_fetched(0) && m.is_fetched(100));
    for row in [0, 99, 100, 101, 199] {
        assert_eq!(cell(&model, row, 0).0, row.to_string());
    }
}

#[test]
fn the_window_is_woken_when_rows_arrive() {
    let (source, _) = Numbers::new(1_000);
    let (wake, wakes) = counted_wake();
    let mut model = PagedModel::new(schema(), source, wake).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    let after_count = wakes.load(Ordering::Relaxed);
    assert!(after_count >= 1);
    model.showing(0..10);
    until(&mut model, |m| m.is_fetched(0));
    assert!(wakes.load(Ordering::Relaxed) > after_count);
}

#[test]
fn dropping_the_model_ends_its_thread() {
    let (mut source, _) = Numbers::new(1_000);
    let (dropped, told) = mpsc::channel();
    source.dropped = Some(dropped);
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    drop(model);
    told.recv_timeout(Duration::from_secs(5))
        .expect("the source is dropped with its thread");
}

#[test]
fn the_first_rows_size_the_columns_and_wider_numbers_widen_them() {
    let (source, _) = Numbers::new(1_000_000);
    let mut model = model(source, 100);
    let before = model.columns_revision();
    assert_eq!(model.columns()[1].title, "name");
    until(&mut model, |m| m.row_count().is_some());
    assert_eq!(model.columns_revision(), before, "no rows, no sizes");

    model.showing(500..510);
    until(&mut model, |m| m.is_fetched(500));
    let sized = model.columns_revision();
    assert_ne!(sized, before, "the first rows size the columns");
    // "row 599": seven characters, wider than the title (the default width holds only until
    // rows arrive).
    assert_eq!(model.columns()[1].chars, 7);
    let narrow = model.columns()[0].chars;
    model.showing(999_900..999_910);
    until(&mut model, |m| m.is_fetched(999_999));
    assert_ne!(model.columns_revision(), sized);
    assert!(model.columns()[0].chars > narrow, "999999 needs more room");
}

#[test]
fn decimals_and_widths_the_app_sets_hold_for_every_page() {
    let (source, _) = Numbers::new(1_000);
    let mut model = model(source, 100);
    assert_eq!(model.page_rows(), 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| m.is_fetched(5));
    assert_eq!(cell(&model, 5, 2).0, "1.6666666666666667");

    let revision = model.columns_revision();
    model.set_decimals(2, Some(2)).unwrap();
    assert_eq!(cell(&model, 5, 2).0, "1.67");
    assert_ne!(model.columns_revision(), revision, "the grid hears of it");
    // Sized again as now printed: "66.33" (pages 0 and 1) and the title at most.
    assert_eq!(model.columns()[2].chars, 5);

    model.set_column_chars(1, 3).unwrap();
    assert_eq!(model.columns()[1].chars, 3);
    assert_eq!(cell(&model, 39, 1).0, "ro…");

    // Pages fetched later keep both.
    model.showing(900..910);
    until(&mut model, |m| m.is_fetched(905));
    assert_eq!(cell(&model, 905, 2).0, "301.67");
    assert_eq!(model.columns()[1].chars, 3, "a width the app set stays");
    assert_eq!(
        model.set_decimals(3, None),
        Err(TableError::NoSuchColumn(3))
    );
    assert_eq!(
        model.set_column_chars(3, 4),
        Err(TableError::NoSuchColumn(3))
    );
}

#[test]
fn a_copy_holds_the_rows_held() {
    let (source, _) = Numbers::new(1_000_000);
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| m.is_fetched(0) && m.is_fetched(100));
    model.showing(500_000..500_010);
    until(&mut model, |m| {
        m.is_fetched(500_000) && m.is_fetched(500_100)
    });
    assert_eq!(
        model.copy_rows(0..1_000_000),
        [0..200, 499_900..500_200],
        "pages held, side by side joined"
    );
    assert_eq!(model.copy_rows(150..499_950), [150..200, 499_900..499_950]);
    assert!(model.copy_rows(300..400).is_empty());
}

#[test]
fn a_page_asked_and_on_its_way_is_not_asked_again() {
    let (source, calls) = Numbers::new(1_000);
    let (source, go) = source.gated();
    let (wake, wakes) = counted_wake();
    let options = PageOptions::default().page_rows(10);
    let mut model = PagedModel::with_options(schema(), source, wake, options).unwrap();
    go.send(()).unwrap(); // the count
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10); // pages 0 and 1, in one call
    go.send(()).unwrap(); // they come back...
    until_woken(&wakes, 2);
    // ...and as the grid does every frame, before they are taken in:
    for _ in 0..5 {
        model.showing(0..10);
    }
    for _ in 0..6 {
        go.send(()).unwrap();
    }
    until(&mut model, |m| m.is_fetched(0) && m.is_fetched(10));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(pages_asked(&calls, 10), [0, 1], "each page fetched once");
}

#[test]
fn a_page_of_the_order_before_a_sort_is_dropped() {
    let (source, _) = Numbers::new(1_000_000);
    let (source, go) = source.gated();
    let (wake, wakes) = counted_wake();
    let options = PageOptions::default().page_rows(100);
    let mut model = PagedModel::with_options(schema(), source, wake, options).unwrap();
    go.send(()).unwrap(); // the count
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    go.send(()).unwrap(); // pages 0 and 1, in the source's order, are sent...
    until_woken(&wakes, 2);
    assert!(model.sort(0, SortOrder::Descending)); // ...but the order changes first
    model.prepare();
    assert!(!model.is_fetched(0), "a page of the old order is not shown");
    assert_eq!(cell(&model, 0, 0).0, "");
    for _ in 0..8 {
        go.send(()).unwrap();
    }
    frames(&mut model, 0..10, |m| m.is_fetched(0));
    assert_eq!(cell(&model, 0, 0).0, "999999");
}

#[test]
fn a_query_says_its_order() {
    let query = PageQuery::new().sorted_by(SortKey::new("n", SortOrder::Descending));
    assert_eq!(query.sort().len(), 1);
    assert_eq!(query.sort()[0].column(), "n");
    assert!(query.sort()[0].is_descending());
    assert_eq!(query.sort()[0].order(), SortOrder::Descending);
    assert!(PageQuery::new().sort().is_empty(), "the source's own order");
}

#[test]
fn a_query_says_its_order_in_sql_with_keys_breaking_ties() {
    let none: [&str; 0] = [];
    assert_eq!(
        PageQuery::new().order_by_sql(&none),
        "",
        "the engine's order"
    );
    assert_eq!(PageQuery::new().order_by_sql(&["id"]), r#"ORDER BY "id""#);
    let by_time = PageQuery::new().sorted_by(SortKey::new("time", SortOrder::Descending));
    assert_eq!(
        by_time.order_by_sql(&["id", "region"]),
        r#"ORDER BY "time" DESC, "id", "region""#
    );
    // A key sorted by already, or named twice, is named once: the first way it comes.
    let by_id = PageQuery::new().sorted_by(SortKey::new("id", SortOrder::Descending));
    assert_eq!(by_id.order_by_sql(&["id", "id"]), r#"ORDER BY "id" DESC"#);
    // Quotes in a name are doubled, as standard SQL does.
    let quoted = PageQuery::new().sorted_by(SortKey::new(r#"a "b""#, SortOrder::Ascending));
    assert_eq!(quoted.order_by_sql(&none), r#"ORDER BY "a ""b""""#);
}

#[test]
fn pages_wanted_below_those_held_come_in_one_call() {
    let (source, calls) = Numbers::new(1_000_000);
    let mut model = model(source, 10);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(1_000..1_010); // pages 99 to 101
    until(&mut model, |m| m.is_fetched(990) && m.is_fetched(1_010));
    let before = page_calls(&calls).len();
    model.showing(970..1_010); // pages 96 to 101: 96, 97 and 98 below those held
    until(&mut model, |m| m.is_fetched(960) && m.is_fetched(980));
    assert_eq!(page_calls(&calls).len() - before, 1);
}

#[test]
fn the_pages_nearest_the_middle_of_those_wanted_come_first() {
    let (source, calls) = Numbers::new(1_000);
    let mut model = model(source, 10);
    until(&mut model, |m| m.row_count().is_some());
    // Rows 10..90: pages 1 to 8, and 0 and 9 beside them. The middle, 4, first with the
    // three after it in one call; then the nearer of what is left, 3, with those below it.
    model.showing(10..90);
    until(&mut model, |m| {
        (0..100).step_by(10).all(|row| m.is_fetched(row))
    });
    assert_eq!(pages_asked(&calls, 10), [4, 5, 6, 7, 0, 1, 2, 3, 8, 9]);
}

#[test]
fn pages_queued_above_those_wanted_are_skipped() {
    let (source, calls) = Numbers::new(1_000_000);
    let (source, go) = source.gated();
    let mut model = model(source, 10);
    go.send(()).unwrap(); // the count
    until(&mut model, |m| m.row_count().is_some());
    model.showing(20_000..20_010); // pages 1999 to 2001: one call, held at the gate
    model.showing(15_000..15_010); // pages 1499 to 1501, queued behind it
    model.showing(0..10); // pages 0 and 1: what is wanted now, below the queued ones
    for _ in 0..6 {
        go.send(()).unwrap();
    }
    until(&mut model, |m| m.is_fetched(0) && m.is_fetched(10));
    std::thread::sleep(Duration::from_millis(50));
    model.prepare();
    let asked = pages_asked(&calls, 10);
    assert!(
        !asked.iter().any(|page| (1_499..=1_501).contains(page)),
        "{asked:?}"
    );
}

/// Rows numbered up to the most a `u64` counts: `n`, the row.
struct Far;

impl PageSource for Far {
    fn row_count(&mut self, _: &PageQuery) -> Result<u64, PageError> {
        Ok(u64::MAX)
    }

    fn page(
        &mut self,
        _: &PageQuery,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, PageError> {
        let end = offset.saturating_add(limit as u64);
        let n: ArrayRef = Arc::new(UInt64Array::from((offset..end).collect::<Vec<_>>()));
        Ok(vec![RecordBatch::try_from_iter([("n", n)])?])
    }
}

#[cfg(target_pointer_width = "64")]
#[test]
fn the_last_rows_of_the_most_there_can_be_come_back() {
    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::UInt64, false)]));
    let options = PageOptions::default().page_rows(1);
    let mut model = PagedModel::with_options(schema, Far, || {}, options).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    let last = usize::MAX - 1;
    frames(&mut model, last - 2..last + 1, |m| m.is_fetched(last));
    assert_eq!(cell(&model, last, 0).0, last.to_string());
    assert_eq!(model.error(), None);
}
