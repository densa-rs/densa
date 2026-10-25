//! Failures, counts, refreshes and retries: what the model shows and asks again when the
//! source fails, gives fewer rows than counted, or the data changes.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use densa_arrow::{FetchError, PageOptions, PagedModel};
use densa_grid::{GridModel, SortOrder};

use crate::support::*;

#[test]
fn a_page_that_fails_says_why_and_is_asked_again_only_on_a_retry() {
    // One page of rows: its call holds no other page.
    let (mut source, calls) = Numbers::new(100);
    source.failing.insert(0);
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| m.error().is_some());
    assert_eq!(
        model.error(),
        Some(&FetchError::Rows {
            rows: 0..100,
            message: "the disk is on fire".to_owned()
        })
    );
    assert_eq!(
        error(&model),
        "rows 0 to 99 could not be read: the disk is on fire"
    );
    assert_eq!(cell(&model, 0, 0), (String::new(), false));

    // Not asked again frame after frame.
    for _ in 0..20 {
        model.showing(0..10);
        model.prepare();
        std::thread::sleep(Duration::from_millis(5));
    }
    model.prepare();
    assert_eq!(page_calls(&calls).len(), 1);
    assert_eq!(cell(&model, 0, 0), (String::new(), false));

    model.retry();
    assert_eq!(model.error(), None);
    frames(&mut model, 0..10, |m| m.is_fetched(0));
    assert_eq!(cell(&model, 0, 0).0, "0");
    assert_eq!(counts(&calls), 1, "a retry of pages counts nothing");
}

#[test]
fn a_page_the_source_cannot_give_takes_none_beside_it_along() {
    let (mut source, calls) = Numbers::new(1_000);
    source.unreadable = Some(50..60);
    let mut model = model(source, 10);
    until(&mut model, |m| m.row_count().is_some());
    // Rows 60..70: page 6, and 5 and 7 beside it, asked in one call that fails for 5.
    model.showing(60..70);
    until(&mut model, |m| {
        m.is_fetched(60) && m.is_fetched(70) && m.error().is_some()
    });
    assert_eq!(cell(&model, 65, 0).0, "65");
    assert!(!model.is_fetched(50));
    // The call of all three, then each alone: the middle first.
    assert_eq!(page_calls(&calls), [(50, 30), (60, 10), (50, 10), (70, 10)]);
    assert_eq!(
        error(&model),
        "rows 50 to 59 could not be read: rows 50 to 59 are unreadable"
    );
}

#[test]
fn rows_that_come_back_short_are_not_held_and_say_to_refresh() {
    let (mut source, calls) = Numbers::new(1_000);
    // As if the data had shrunk since it was counted.
    source.most = Some(150);
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| m.is_fetched(0) && m.error().is_some());
    assert_eq!(
        model.error(),
        Some(&FetchError::Short {
            rows: 100..200,
            got: 50
        })
    );
    assert!(error(&model).contains("refresh"), "{}", error(&model));
    assert!(!model.is_fetched(100));
    assert_eq!(cell(&model, 120, 0), (String::new(), false));
    // Not asked again: shown with the page after it, that page alone is fetched.
    frames(&mut model, 0..150, |m| m.is_fetched(200));
    assert_eq!(page_calls(&calls), [(0, 200), (200, 100)]);
    assert!(!model.is_fetched(100));
}

#[test]
fn a_source_that_panics_is_reported_and_its_thread_goes_on() {
    let (mut source, _) = Numbers::new(100);
    source.panicking.insert(0);
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| m.error().is_some());
    let error = error(&model);
    assert!(
        error.contains("panicked") && error.contains("page 0 blew up"),
        "{error}"
    );
    model.retry();
    frames(&mut model, 0..10, |m| m.is_fetched(0));
    assert_eq!(cell(&model, 0, 0).0, "0");
}

#[test]
fn a_page_of_other_columns_is_refused_naming_the_column() {
    let (mut source, _) = Numbers::new(1_000);
    source.wrong = true;
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| m.error().is_some());
    let error = error(&model);
    assert!(
        error.contains("column n") && error.contains("Utf8") && error.contains("Int64"),
        "{error}"
    );
    assert!(!model.is_fetched(0));
}

#[test]
fn a_page_of_fewer_columns_is_refused_saying_so() {
    let (mut source, _) = Numbers::new(1_000);
    source.narrow = true;
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| m.error().is_some());
    let error = error(&model);
    assert!(
        error.contains("1 columns") && error.contains("has 3"),
        "{error}"
    );
    assert!(!model.is_fetched(0));
}

#[test]
fn a_count_that_fails_shows_no_rows_says_why_and_is_retried() {
    let (mut source, calls) = Numbers::new(1_000);
    source.failing_counts.insert(0);
    let mut model = model(source, 100);
    until(&mut model, |m| m.error().is_some());
    assert_eq!(model.rows(), 0);
    assert_eq!(model.row_count(), None);
    assert_eq!(
        error(&model),
        "the rows could not be counted: the count is off today"
    );
    model.showing(0..10);
    model.prepare();
    assert!(page_calls(&calls).is_empty(), "no rows, no pages");

    model.retry();
    until(&mut model, |m| m.row_count().is_some());
    assert_eq!(model.row_count(), Some(1_000));
    assert_eq!(model.error(), None);
}

#[test]
fn a_refresh_shows_the_rows_held_until_the_new_ones_arrive() {
    let (source, calls) = Numbers::new(1_000);
    let (source, go) = source.gated();
    let mut model = model(source, 100);
    go.send(()).unwrap(); // the count
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    go.send(()).unwrap();
    until(&mut model, |m| m.is_fetched(0));

    model.refresh();
    assert_eq!(cell(&model, 5, 0).0, "5", "still shown");
    assert!(!model.is_fetched(5), "but to be fetched again");
    model.showing(0..10);
    model.prepare();
    assert_eq!(
        page_calls(&calls).len(),
        1,
        "no page asked before the new count"
    );
    go.send(()).unwrap(); // the new count
    go.send(()).unwrap(); // the page after it
    frames(&mut model, 0..10, |m| m.is_fetched(5));
    assert_eq!(cell(&model, 5, 0).0, "5");
    assert_eq!(counts(&calls), 2);
}

#[test]
fn a_refresh_after_the_rows_shrank_asks_for_none_past_them() {
    let (mut source, calls) = Numbers::new(1_000);
    source.counts = VecDeque::from([1_000, 100]);
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(900..940);
    until(&mut model, |m| m.is_fetched(900));
    let before = calls.lock().unwrap().len();

    model.refresh();
    // The grid still shows the old count until the new one is in.
    model.showing(900..940);
    until(&mut model, |m| m.row_count() == Some(100));
    // As the grid shows them now: the last rows there are.
    frames(&mut model, 60..100, |m| m.is_fetched(60));
    assert_eq!(model.error(), None);
    let after: Vec<Call> = calls.lock().unwrap()[before..].to_vec();
    assert!(
        after
            .iter()
            .all(|call| !matches!(call, Call::Page { offset, .. } if *offset >= 100)),
        "{after:?}"
    );
}

#[test]
fn a_count_from_before_a_refresh_is_dropped() {
    let (mut source, calls) = Numbers::new(2_000);
    source.counts = VecDeque::from([1_000, 2_000]);
    let (source, go) = source.gated();
    let (wake, wakes) = counted_wake();
    let mut model = PagedModel::new(schema(), source, wake).unwrap();
    until_counting(&calls, 1);
    model.refresh(); // while the first count is held at the gate
    go.send(()).unwrap(); // the first count: 1,000
    until_woken(&wakes, 1);
    model.prepare();
    assert_eq!(
        model.row_count(),
        None,
        "the count asked before the refresh"
    );
    go.send(()).unwrap(); // the count after: 2,000
    until(&mut model, |m| m.row_count().is_some());
    assert_eq!(model.row_count(), Some(2_000));
}

#[test]
fn a_sort_while_the_rows_are_counted_takes_that_count() {
    let (source, calls) = Numbers::new(1_000);
    let (source, go) = source.gated();
    let mut model = model(source, 100);
    until_counting(&calls, 1);
    // Sorted while the count is held at the gate: a new order has as many rows.
    assert!(model.sort(0, SortOrder::Descending));
    for _ in 0..4 {
        go.send(()).unwrap();
    }
    until(&mut model, |m| m.row_count().is_some());
    frames(&mut model, 0..10, |m| m.is_fetched(0));
    assert_eq!(cell(&model, 0, 0).0, "999");
    assert_eq!(counts(&calls), 1, "counted once");
}

#[test]
fn a_count_asked_with_a_sort_after_it_is_taken_in_the_new_order() {
    let (source, calls) = Numbers::new(1_000);
    let (source, go) = source.gated();
    let mut model = model(source, 100);
    go.send(()).unwrap(); // the count
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    // Pages 0 and 1, held at the gate. Meanwhile a refresh asks for a count and a sort for
    // none: the thread takes both at once and counts the newest query, the sorted one.
    until_paging(&calls, 1);
    model.refresh();
    assert!(model.sort(0, SortOrder::Descending));
    for _ in 0..4 {
        go.send(()).unwrap();
    }
    frames(&mut model, 0..10, |m| m.is_fetched(0));
    assert_eq!(cell(&model, 0, 0).0, "999");
    let descending = Some(("n".to_owned(), SortOrder::Descending));
    assert_eq!(calls.lock().unwrap()[2], Call::Count(descending));
    assert_eq!(counts(&calls), 2);
}

#[test]
fn a_sort_after_the_count_failed_counts_again() {
    let (mut source, calls) = Numbers::new(1_000);
    source.failing_counts.insert(0);
    let mut model = model(source, 100);
    until(&mut model, |m| m.error().is_some());
    assert_eq!(model.row_count(), None);
    // Another query: counted again, as nothing else would.
    assert!(model.sort(0, SortOrder::Descending));
    until(&mut model, |m| m.row_count().is_some());
    assert_eq!(model.error(), None);
    frames(&mut model, 0..10, |m| m.is_fetched(0));
    assert_eq!(cell(&model, 0, 0).0, "999");
    assert_eq!(counts(&calls), 2);
}

#[test]
fn a_retry_while_the_rows_are_counted_does_not_count_again() {
    let (source, calls) = Numbers::new(1_000);
    let (source, go) = source.gated();
    let mut model = model(source, 100);
    until_counting(&calls, 1);
    model.retry();
    for _ in 0..4 {
        go.send(()).unwrap();
    }
    until(&mut model, |m| m.row_count().is_some());
    frames(&mut model, 0..10, |m| m.is_fetched(0));
    assert_eq!(counts(&calls), 1, "counted once");
}

#[test]
fn a_thread_that_stops_says_why_and_nothing_clears_it() {
    let (mut source, _) = Numbers::new(1_000);
    source.unreadable = Some(0..100);
    let wakes = Arc::new(AtomicUsize::new(0));
    let woken = Arc::clone(&wakes);
    // The second wake (after page 0's failure is sent) panics, stopping the thread.
    let wake = move || {
        if woken.fetch_add(1, Ordering::Relaxed) == 1 {
            panic!("the window is gone");
        }
    };
    let options = PageOptions::default().page_rows(100);
    let mut model = PagedModel::with_options(schema(), source, wake, options).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| {
        matches!(m.error(), Some(FetchError::Stopped { .. }))
    });
    assert_eq!(
        error(&model),
        "the thread fetching pages stopped (a panic: the window is gone): make the model again"
    );
    model.retry();
    model.refresh();
    assert!(model.sort(0, SortOrder::Descending));
    for _ in 0..5 {
        model.prepare();
        model.showing(0..10);
    }
    assert!(matches!(model.error(), Some(FetchError::Stopped { .. })));
}

#[test]
fn a_refresh_whose_count_fails_shows_no_rows() {
    let (mut source, _) = Numbers::new(1_000);
    source.failing_counts.insert(1);
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until(&mut model, |m| m.is_fetched(0));
    model.refresh();
    until(&mut model, |m| m.error().is_some());
    assert_eq!(model.row_count(), None);
    assert_eq!(model.rows(), 0, "rows that may be gone are not shown");
}
