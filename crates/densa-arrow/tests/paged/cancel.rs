//! Calls the model no longer wants are cancelled: what they give is not used, their pages are
//! asked for again when shown again, and a source that looks ends them early.

use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};
use std::time::Duration;

use arrow_array::RecordBatch;
use densa_arrow::{PageCall, PageError, PageQuery, PageSource, PagedModel};
use densa_grid::{GridModel, SortOrder};

use super::support::*;

/// A source of a million rows that looks, with what tells how its calls go.
struct Looking {
    source: Numbers,
    calls: Arc<Mutex<Vec<Call>>>,
    /// Lets the call waiting at the gate go on.
    go: Sender<()>,
    /// Calls that ended cancelled.
    stops: Arc<AtomicUsize>,
}

/// A source of a million rows that looks: each call waits at a gate or until it is cancelled.
fn looking() -> Looking {
    let (mut source, calls) = Numbers::new(1_000_000);
    source.looks = true;
    let stops = source.stops();
    let (source, go) = source.gated();
    Looking {
        source,
        calls,
        go,
        stops,
    }
}

#[test]
fn a_call_whose_pages_scroll_out_of_view_is_cancelled_and_they_are_asked_again_when_shown() {
    let Looking {
        source,
        calls,
        go,
        stops,
    } = looking();
    let mut model = model(source, 100);
    go.send(()).unwrap(); // the count
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until_paging(&calls, 1);

    // Far down while pages 0 and 1 are on their way: that call ends, the pages there come.
    model.showing(500_000..500_010);
    until_stopped(&stops, 1);
    until_paging(&calls, 2);
    go.send(()).unwrap();
    frames(&mut model, 500_000..500_010, |m| m.is_fetched(500_000));
    assert!(!model.is_fetched(0));
    assert_eq!(model.error(), None, "a cancel is no failure");

    // Back up: pages 0 and 1 are asked for again.
    go.send(()).unwrap();
    frames(&mut model, 0..10, |m| m.is_fetched(0) && m.is_fetched(100));
    assert_eq!(cell(&model, 5, 0).0, "5");
    assert_eq!(
        page_calls(&calls),
        [(0, 200), (499_900, 300), (0, 200)],
        "each call whole: none split into calls of one page"
    );
    assert_eq!(stops.load(Ordering::Relaxed), 1);
}

#[test]
fn a_call_with_a_page_still_in_view_goes_on() {
    let Looking {
        source,
        calls,
        go,
        stops,
    } = looking();
    let mut model = model(source, 100);
    go.send(()).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until_paging(&calls, 1);
    // Pages 0 to 2 wanted now: the call of 0 and 1 is still wanted.
    model.showing(150..160);
    for _ in 0..2 {
        go.send(()).unwrap();
    }
    frames(&mut model, 150..160, |m| {
        m.is_fetched(0) && m.is_fetched(200)
    });
    assert_eq!(page_calls(&calls), [(0, 200), (200, 100)]);
    assert_eq!(stops.load(Ordering::Relaxed), 0);
}

#[test]
fn a_cancelled_call_of_one_page_is_no_failure() {
    let Looking {
        mut source,
        calls,
        go,
        stops,
    } = looking();
    source.failing.insert(0);
    let mut model = model(source, 100);
    go.send(()).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    // Pages 0 and 1 fail together, then each is asked for alone: page 0 first.
    go.send(()).unwrap();
    until_paging(&calls, 2);
    assert_eq!(page_calls(&calls)[1], (0, 100));

    model.showing(500_000..500_010);
    until_stopped(&stops, 1);
    go.send(()).unwrap();
    frames(&mut model, 500_000..500_010, |m| m.is_fetched(500_000));
    assert_eq!(model.error(), None);

    // Page 0 is not taken for one the source failed to give: shown again, it is asked again.
    go.send(()).unwrap();
    frames(&mut model, 0..10, |m| m.is_fetched(0));
    assert_eq!(model.error(), None);
}

#[test]
fn a_sort_cancels_the_call_of_the_old_order_and_counts_nothing() {
    let Looking {
        source,
        calls,
        go,
        stops,
    } = looking();
    let mut model = model(source, 100);
    go.send(()).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until_paging(&calls, 1);
    assert!(model.sort(0, SortOrder::Descending));
    until_stopped(&stops, 1);
    go.send(()).unwrap();
    frames(&mut model, 0..10, |m| m.is_fetched(0));
    assert_eq!(cell(&model, 0, 0).0, "999999");
    assert_eq!(counts(&calls), 1);
}

#[test]
fn a_sort_keeps_the_count_on_its_way() {
    let Looking {
        source,
        calls,
        go,
        stops,
    } = looking();
    let mut model = model(source, 100);
    until_counting(&calls, 1);
    assert!(model.sort(0, SortOrder::Descending));
    go.send(()).unwrap();
    // Cancelled, this count would end failing and no other would be asked: none would come.
    until(&mut model, |m| m.row_count().is_some());
    assert_eq!(counts(&calls), 1);
    assert_eq!(stops.load(Ordering::Relaxed), 0);
}

#[test]
fn a_refresh_cancels_the_count_on_its_way_and_takes_the_next() {
    let Looking {
        source,
        calls,
        go,
        stops,
    } = looking();
    let mut model = model(source, 100);
    until_counting(&calls, 1);
    model.refresh();
    until_stopped(&stops, 1);
    go.send(()).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    assert_eq!(counts(&calls), 2);
    assert_eq!(model.error(), None, "a cancel is no failure");
}

#[test]
fn rows_a_cancelled_call_gives_whole_are_kept() {
    // A source that does not look: its call runs to its end, and its pages are held.
    let (source, calls) = Numbers::new(1_000_000);
    let (source, go) = source.gated();
    let mut model = model(source, 100);
    go.send(()).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until_paging(&calls, 1);
    model.showing(500_000..500_010);
    for _ in 0..2 {
        go.send(()).unwrap();
    }
    frames(&mut model, 500_000..500_010, |m| m.is_fetched(500_000));
    assert!(model.is_fetched(0) && model.is_fetched(100));
    assert_eq!(model.pages_held(), 5);
    // Back up: nothing to ask for.
    frames(&mut model, 0..10, |m| m.is_fetched(0));
    assert_eq!(page_calls(&calls), [(0, 200), (499_900, 300)]);
}

#[test]
fn a_cancelled_call_that_stops_short_is_no_failure() {
    let Looking {
        mut source,
        calls,
        go,
        stops,
    } = looking();
    source.stops_short = true;
    let mut model = model(source, 100);
    go.send(()).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until_paging(&calls, 1);
    // Cancelled, the call gives half its rows: page 0 whole, page 1 none.
    model.showing(500_000..500_010);
    until_stopped(&stops, 1);
    go.send(()).unwrap();
    frames(&mut model, 500_000..500_010, |m| m.is_fetched(500_000));
    assert!(model.is_fetched(0), "a page given whole is kept");
    assert!(!model.is_fetched(100));
    assert_eq!(model.error(), None, "a page short is no failure");
    // Back up: page 1 alone is asked for again.
    go.send(()).unwrap();
    frames(&mut model, 0..10, |m| m.is_fetched(100));
    assert_eq!(page_calls(&calls), [(0, 200), (499_900, 300), (100, 100)]);
    assert_eq!(model.error(), None);
}

#[test]
fn quick_jumps_against_a_source_that_looks_lose_no_rows() {
    let (mut source, _) = Numbers::new(1_000_000);
    source.latency = Some(Duration::from_millis(3));
    let stops = source.stops();
    let mut model = model(source, 100);
    until(&mut model, |m| m.row_count().is_some());
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut jump = || {
        // xorshift: places across the rows.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        usize::try_from(state % 999_960).unwrap()
    };
    for _ in 0..40 {
        // A few frames, each far from the last, then one that rests.
        for _ in 0..3 {
            let at = jump();
            model.prepare();
            model.showing(at..at + 40);
            std::thread::sleep(Duration::from_millis(1));
        }
        let at = jump();
        frames(&mut model, at..at + 40, |m| {
            m.is_fetched(at) && m.is_fetched(at + 39)
        });
        assert_eq!(cell(&model, at + 39, 0).0, (at + 39).to_string());
        assert_eq!(model.error(), None);
    }
    assert!(
        stops.load(Ordering::Relaxed) > 0,
        "calls were cancelled as the view jumped"
    );
}

#[test]
fn dropping_the_model_cancels_its_call() {
    let Looking {
        mut source,
        calls,
        go,
        stops,
    } = looking();
    let (dropped, gone) = mpsc::channel();
    source.dropped = Some(dropped);
    let mut model = model(source, 100);
    go.send(()).unwrap();
    until(&mut model, |m| m.row_count().is_some());
    model.showing(0..10);
    until_paging(&calls, 1);
    drop(model);
    // The gate stays shut (`go` lives on): only the cancel ends the call, then the thread.
    gone.recv_timeout(Duration::from_secs(5))
        .expect("the source is dropped once its call ends");
    assert_eq!(stops.load(Ordering::Relaxed), 1);
    drop(go);
}

/// How a [`Waits`] call waits.
#[derive(Debug, Clone, Copy)]
enum Wait {
    /// [`PageCall::cancelled_within`].
    Within,
    /// [`PageCall::cancelled`], awaited.
    Await,
}

/// A source whose page calls say they began, wait until they are cancelled, then say so.
struct Waits {
    how: Wait,
    began: Sender<()>,
    ended: Sender<bool>,
}

impl PageSource for Waits {
    fn row_count(&mut self, _: &PageQuery) -> Result<u64, PageError> {
        Ok(1_000_000)
    }

    fn page(&mut self, _: &PageQuery, _: u64, _: usize) -> Result<Vec<RecordBatch>, PageError> {
        let call = PageCall::current();
        let _ = self.began.send(());
        let cancelled = match self.how {
            Wait::Within => call.cancelled_within(Duration::from_secs(30)),
            Wait::Await => {
                block_on(call.cancelled());
                true
            }
        };
        let _ = self.ended.send(cancelled && call.is_cancelled());
        Err("cancelled".into())
    }
}

/// Wakes a thread parked in [`block_on`].
struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Runs `future` on this thread until it is done.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        thread::park();
    }
}

#[test]
fn a_call_waiting_on_its_cancel_or_awaiting_it_ends_when_cancelled() {
    for how in [Wait::Within, Wait::Await] {
        let (began, calls_began) = mpsc::channel();
        let (ended, calls_ended) = mpsc::channel();
        let source = Waits { how, began, ended };
        let mut model = PagedModel::new(schema(), source, || {}).unwrap();
        until(&mut model, |m| m.row_count().is_some());
        model.showing(0..10);
        calls_began.recv_timeout(Duration::from_secs(5)).unwrap();
        model.showing(500_000..500_010);
        let cancelled = calls_ended
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| panic!("{how:?}: the call ends once cancelled"));
        assert!(cancelled, "{how:?}");
        // The next call waits too: dropping the model ends it.
        calls_began.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(model);
        assert_eq!(
            calls_ended.recv_timeout(Duration::from_secs(5)),
            Ok(true),
            "{how:?}"
        );
    }
}
