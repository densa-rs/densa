//! The source call running, and whether the model still wants what it asks: the model cancels a
//! call it no longer wants, and a source that can stop early learns of it through a
//! [`PageCall`].

use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::mem;
use std::ops::Range;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

/// What a call asks the source for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Asked {
    /// The rows of the query of `epoch`, counted.
    Count { epoch: u64 },
    /// Pages `pages` of the query of `epoch`.
    Pages { epoch: u64, pages: Range<u64> },
}

/// What the model wants, and the call its thread is in.
#[derive(Debug, Default)]
struct State {
    /// The model's epoch: pages of another are not wanted.
    epoch: u64,
    /// The epoch the last count was asked in: a count of an earlier one is not wanted.
    count_epoch: u64,
    /// The first and last pages wanted.
    first: u64,
    last: u64,
    /// Whether the model is gone: nothing is wanted.
    dropped: bool,
    /// The number of the call running, or of the last: calls are numbered from 1.
    serial: u64,
    /// What the call running asks; `None` between calls.
    asked: Option<Asked>,
    /// Futures waiting for the call running to be over.
    wakers: Vec<Waker>,
}

impl State {
    /// Whether the model wants what `asked` asks for.
    fn is_wanted(&self, asked: &Asked) -> bool {
        if self.dropped {
            return false;
        }
        match asked {
            Asked::Count { epoch } => answers_count(*epoch, self.count_epoch, self.epoch),
            Asked::Pages { epoch, pages } => {
                *epoch == self.epoch && self.first < pages.end && pages.start <= self.last
            }
        }
    }
}

/// What the model wants now, and the call its thread is in: shared by the two. The model says
/// what it wants and cancels the call running if that no longer is; the thread begins a call
/// cancelled if the model changed its mind as it chose it.
#[derive(Debug, Default)]
pub(super) struct Calls {
    /// Calls up to this number are over: ended or cancelled. Written under the lock, read
    /// without it by those that only look.
    over: AtomicU64,
    state: Mutex<State>,
    /// Told when a call is over, for those waiting in [`PageCall::cancelled_within`].
    ended: Condvar,
}

impl Calls {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Under the lock run this file's code, which does not panic, and wakers' clones: one
        // that panics leaves the state whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether call `serial` is over.
    fn is_over(&self, serial: u64) -> bool {
        // Relaxed: nothing is published through `over`. A look outside the lock sees a cancel
        // soon enough; those that wait read it under the lock it is written under.
        self.over.load(Ordering::Relaxed) >= serial
    }

    /// Marks the calls up to `serial` over; under the lock.
    fn set_over(&self, serial: u64) {
        // Relaxed: see `is_over`.
        self.over.fetch_max(serial, Ordering::Relaxed);
    }

    /// Changes what the model wants, then cancels the call running if that is no longer wanted.
    fn want(&self, change: impl FnOnce(&mut State)) {
        let wakers = {
            let mut state = self.lock();
            change(&mut state);
            match &state.asked {
                // A call cancelled already is so again: nothing waits for it.
                Some(asked) if !state.is_wanted(asked) => {
                    self.set_over(state.serial);
                    mem::take(&mut state.wakers)
                }
                _ => return,
            }
        };
        self.tell(wakers);
    }

    /// Says the model is in epoch `epoch`, the last count asked in `count_epoch`.
    pub(super) fn want_epochs(&self, epoch: u64, count_epoch: u64) {
        self.want(|state| {
            state.epoch = epoch;
            state.count_epoch = count_epoch;
        });
    }

    /// Says pages `first..=last` are wanted (`first <= last`).
    pub(super) fn want_pages(&self, first: u64, last: u64) {
        self.want(|state| {
            state.first = first;
            state.last = last;
        });
    }

    /// Says the model is gone.
    pub(super) fn drop_model(&self) {
        self.want(|state| state.dropped = true);
    }

    /// The first and last pages wanted.
    pub(super) fn wanted(&self) -> (u64, u64) {
        let state = self.lock();
        (state.first, state.last)
    }

    /// Wakes those waiting for a call now over.
    fn tell(&self, wakers: Vec<Waker>) {
        self.ended.notify_all();
        // Outside the lock: a waker may poll its future at once, which takes the lock.
        wakers.into_iter().for_each(Waker::wake);
    }

    /// Begins a call asking `asked`: the call, cancelled already if the model no longer wants
    /// it (it changed its mind as the thread chose it). Calls are one at a time.
    pub(super) fn begin(self: &Arc<Self>, asked: Asked) -> PageCall {
        let mut state = self.lock();
        state.serial += 1;
        if !state.is_wanted(&asked) {
            self.set_over(state.serial);
        }
        state.asked = Some(asked);
        PageCall {
            calls: Some(Arc::clone(self)),
            serial: state.serial,
        }
    }

    /// Ends `call`: whether it was cancelled.
    pub(super) fn end(&self, call: &PageCall) -> bool {
        let (cancelled, wakers) = {
            let mut state = self.lock();
            state.asked = None;
            let cancelled = self.is_over(call.serial);
            self.set_over(call.serial);
            (cancelled, mem::take(&mut state.wakers))
        };
        self.tell(wakers);
        cancelled
    }
}

/// Whether a count of epoch `of` answers the count asked last, in `count_epoch`, with the model
/// in `epoch`: one of `count_epoch` or later (a sort asks for none), not of one before (a
/// refresh asks again).
pub(super) fn answers_count(of: u64, count_epoch: u64, epoch: u64) -> bool {
    of.wrapping_sub(count_epoch) <= epoch.wrapping_sub(count_epoch)
}

thread_local! {
    /// The call this thread is in, while a model's thread calls its source.
    static CURRENT: RefCell<PageCall> = const { RefCell::new(PageCall::NONE) };
}

/// Makes a call this thread's current one until dropped (when the call returns or panics).
pub(super) struct Current;

impl Current {
    pub(super) fn set(call: &PageCall) -> Self {
        CURRENT.with(|current| *current.borrow_mut() = call.clone());
        Self
    }
}

impl Drop for Current {
    fn drop(&mut self) {
        CURRENT.with(|current| *current.borrow_mut() = PageCall::NONE);
    }
}

/// A call of a [`PageSource`](super::PageSource) by a [`PagedModel`](super::PagedModel)'s
/// thread, and whether the model still wants what it asks. The model cancels a call once it no
/// longer does: its pages scrolled out of view, another order or a refresh asked for (a sort
/// keeps a count, which holds for any order), or the model dropped. A source whose calls can
/// stop early (a query engine's stream, a remote query, a wait) then stops, returning an error
/// or fewer rows than asked for: neither is a failure, and the pages are asked for again if
/// they are shown again (rows it returns whole anyway are kept).
///
/// Take it with [`PageCall::current`] in [`row_count`](super::PageSource::row_count) or
/// [`page`](super::PageSource::page), then look ([`is_cancelled`](Self::is_cancelled)), wait on
/// it ([`cancelled_within`](Self::cancelled_within)) or await it
/// ([`cancelled`](Self::cancelled)) as the work goes. It can be cloned and sent to the threads
/// or tasks doing that work. Once the call returns, it says cancelled: its work can stop.
///
/// ```
/// use std::time::Duration;
///
/// use densa_arrow::arrow_array::RecordBatch;
/// use densa_arrow::{PageCall, PageError, PageQuery, PageSource};
///
/// /// A service that runs a query as a job: started, then asked how it goes.
/// struct Jobs;
///
/// impl PageSource for Jobs {
///     fn row_count(&mut self, _: &PageQuery) -> Result<u64, PageError> {
///         Ok(1_000_000)
///     }
///
///     fn page(
///         &mut self,
///         _: &PageQuery,
///         offset: u64,
///         limit: usize,
///     ) -> Result<Vec<RecordBatch>, PageError> {
///         let call = PageCall::current();
///         let job = (offset, limit); // start the job here
///         for _ in 0..3 {
///             // Between looks, a wait that ends as soon as the call is cancelled.
///             if call.cancelled_within(Duration::from_millis(1)) {
///                 // Stop the job here: no one wants its rows, and the error is not shown.
///                 return Err("cancelled".into());
///             }
///             // Is the job done? Then return its rows.
///         }
///         let _ = job;
///         Ok(Vec::new())
///     }
/// }
///
/// // Called directly, not by a model's thread: never cancelled.
/// assert!(!PageCall::current().is_cancelled());
/// assert_eq!(Jobs.page(&PageQuery::new(), 0, 256).unwrap().len(), 0);
/// ```
#[derive(Clone)]
pub struct PageCall {
    /// The calls shared with the model; `None` for no call, never cancelled.
    calls: Option<Arc<Calls>>,
    /// This call's number.
    serial: u64,
}

impl PageCall {
    /// No call: never cancelled.
    const NONE: Self = Self {
        calls: None,
        serial: 0,
    };

    /// The call this thread is in: that of a [`PageSource`](super::PageSource) method, when a
    /// model's thread calls it. Elsewhere (a source called directly, another thread: take it
    /// before handing work to one), a call never cancelled.
    #[must_use]
    pub fn current() -> Self {
        CURRENT.with(|current| current.borrow().clone())
    }

    /// Whether the call was cancelled, or has returned: whether its work can stop.
    pub fn is_cancelled(&self) -> bool {
        self.calls
            .as_ref()
            .is_some_and(|calls| calls.is_over(self.serial))
    }

    /// Waits until the call is cancelled, `timeout` at most: whether it was. For a source that
    /// waits between looks at its work (how a remote job goes, a retry), so a cancel ends the
    /// wait at once. A call never cancelled waits the whole `timeout`.
    pub fn cancelled_within(&self, timeout: Duration) -> bool {
        let Some(calls) = &self.calls else {
            thread::sleep(timeout);
            return false;
        };
        let deadline = Instant::now().checked_add(timeout);
        let mut state = calls.lock();
        loop {
            if calls.is_over(self.serial) {
                return true;
            }
            state = match deadline {
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return false;
                    }
                    calls
                        .ended
                        .wait_timeout(state, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                // Past what an Instant holds: as long as it takes.
                None => calls
                    .ended
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner),
            };
        }
    }

    /// A future done once the call is cancelled: for a source whose work is asynchronous, to
    /// race against it (with `select!` in the runtime it runs on). It is woken on the thread
    /// that cancels the call: the model's, or the one it runs on when the call returns. A call
    /// never cancelled gives one never done.
    pub fn cancelled(&self) -> CallCancelled {
        CallCancelled { call: self.clone() }
    }
}

impl fmt::Debug for PageCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageCall")
            .field("call", &self.calls.as_ref().map(|_| self.serial))
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// A future done once a [`PageCall`] is cancelled, or has returned: what
/// [`PageCall::cancelled`] gives.
#[derive(Debug)]
#[must_use = "futures do nothing unless awaited"]
pub struct CallCancelled {
    call: PageCall,
}

impl Future for CallCancelled {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let Some(calls) = &self.call.calls else {
            return Poll::Pending;
        };
        let mut state = calls.lock();
        if calls.is_over(self.call.serial) {
            return Poll::Ready(());
        }
        // Not over, so the call running: told when it is.
        if !state.wakers.iter().any(|known| known.will_wake(cx.waker())) {
            state.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;

    use super::*;

    fn pages(epoch: u64, pages: Range<u64>) -> Asked {
        Asked::Pages { epoch, pages }
    }

    #[test]
    fn a_call_of_pages_out_of_view_or_of_another_epoch_is_cancelled() {
        let calls = Arc::new(Calls::default());
        calls.want_epochs(3, 1);
        calls.want_pages(10, 12);
        let call = calls.begin(pages(3, 12..16));
        assert!(!call.is_cancelled(), "page 12 is wanted");
        calls.want_pages(9, 11);
        assert!(call.is_cancelled(), "none of 12..16 is wanted");
        assert!(calls.end(&call));

        let call = calls.begin(pages(3, 8..10));
        assert!(!call.is_cancelled(), "page 9 is wanted");
        calls.want_pages(10, 20);
        assert!(call.is_cancelled(), "page 10 is past 8..10");
        assert!(calls.end(&call));

        let call = calls.begin(pages(3, 10..12));
        calls.want_epochs(4, 1);
        assert!(call.is_cancelled(), "another epoch");
        assert!(calls.end(&call));
    }

    #[test]
    fn a_call_already_unwanted_when_it_begins_is_cancelled_at_once() {
        let calls = Arc::new(Calls::default());
        calls.want_epochs(2, 2);
        calls.want_pages(0, 3);
        assert!(calls.begin(pages(1, 0..1)).is_cancelled(), "an old epoch");
        assert!(calls.begin(pages(2, 4..5)).is_cancelled(), "out of view");
        assert!(!calls.begin(pages(2, 3..4)).is_cancelled());
        assert!(calls.begin(Asked::Count { epoch: 1 }).is_cancelled());
        assert!(!calls.begin(Asked::Count { epoch: 2 }).is_cancelled());
    }

    #[test]
    fn a_count_is_cancelled_by_a_newer_count_not_by_a_sort() {
        let calls = Arc::new(Calls::default());
        calls.want_epochs(1, 1);
        let count = calls.begin(Asked::Count { epoch: 1 });
        calls.want_epochs(2, 1);
        assert!(
            !count.is_cancelled(),
            "a sort: the count holds for its order"
        );
        calls.want_epochs(3, 3);
        assert!(count.is_cancelled(), "a refresh counts again");
        assert!(calls.end(&count));
    }

    #[test]
    fn a_call_ended_says_cancelled_but_was_not() {
        let calls = Arc::new(Calls::default());
        let call = calls.begin(pages(0, 0..1));
        let kept = call.clone();
        assert!(!calls.end(&call));
        assert!(kept.is_cancelled(), "its work can stop");
        // Nothing is running: what the model says now cancels nothing.
        calls.drop_model();
        let next = calls.begin(pages(0, 0..1));
        assert!(next.is_cancelled(), "the model is gone");
    }

    #[test]
    fn a_model_dropped_cancels_the_call_running() {
        let calls = Arc::new(Calls::default());
        let call = calls.begin(Asked::Count { epoch: 0 });
        calls.drop_model();
        assert!(call.is_cancelled());
    }

    #[test]
    fn no_call_is_never_cancelled() {
        let call = PageCall::current();
        assert!(!call.is_cancelled());
        assert!(!call.cancelled_within(Duration::from_millis(1)));
        let mut cancelled = std::pin::pin!(call.cancelled());
        let mut cx = Context::from_waker(Waker::noop());
        assert_eq!(cancelled.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(
            format!("{call:?}"),
            "PageCall { call: None, cancelled: false }"
        );
    }

    #[test]
    fn the_current_call_is_the_one_set_until_its_guard_drops() {
        let calls = Arc::new(Calls::default());
        let call = calls.begin(pages(0, 0..1));
        {
            let _current = Current::set(&call);
            calls.drop_model();
            assert!(PageCall::current().is_cancelled());
        }
        assert!(!PageCall::current().is_cancelled(), "no call now");
    }

    #[test]
    fn a_call_that_panics_leaves_no_current_call() {
        let calls = Arc::new(Calls::default());
        let call = calls.begin(pages(0, 0..1));
        let panicked = panic::catch_unwind(AssertUnwindSafe(|| {
            let _current = Current::set(&call);
            panic!("the source blew up");
        }));
        assert!(panicked.is_err());
        assert!(!calls.end(&call));
        // Were the call still current, it would say cancelled: it returned.
        assert!(!PageCall::current().is_cancelled());
    }

    #[test]
    fn a_call_and_its_future_go_to_other_threads() {
        fn sent<T: Send + Sync + 'static>() {}
        sent::<PageCall>();
        sent::<CallCancelled>();
    }

    /// Counts its wakes.
    struct Counted(AtomicUsize);

    impl Wake for Counted {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn the_future_is_woken_once_when_the_call_is_cancelled_or_ends() {
        let calls = Arc::new(Calls::default());
        calls.want_pages(0, 1);
        let counted = Arc::new(Counted(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&counted));
        let mut cx = Context::from_waker(&waker);

        let call = calls.begin(pages(0, 0..1));
        let mut cancelled = std::pin::pin!(call.cancelled());
        assert_eq!(cancelled.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(
            cancelled.as_mut().poll(&mut cx),
            Poll::Pending,
            "polled again"
        );
        calls.want_pages(5, 6);
        assert_eq!(
            counted.0.load(Ordering::Relaxed),
            1,
            "one waker kept, woken once"
        );
        assert_eq!(cancelled.as_mut().poll(&mut cx), Poll::Ready(()));
        assert!(calls.end(&call));

        let call = calls.begin(pages(0, 5..6));
        let mut ended = std::pin::pin!(call.cancelled());
        assert_eq!(ended.as_mut().poll(&mut cx), Poll::Pending);
        assert!(!calls.end(&call));
        assert_eq!(counted.0.load(Ordering::Relaxed), 2, "woken when it ended");
        assert_eq!(ended.as_mut().poll(&mut cx), Poll::Ready(()));
    }

    #[test]
    fn a_wait_ends_when_the_call_is_cancelled_from_another_thread() {
        let calls = Arc::new(Calls::default());
        calls.want_pages(0, 1);
        let call = calls.begin(pages(0, 0..1));
        let model = Arc::clone(&calls);
        let cancel = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            model.want_pages(7, 8);
        });
        let start = Instant::now();
        assert!(call.cancelled_within(Duration::from_secs(30)));
        assert!(start.elapsed() < Duration::from_secs(10));
        cancel.join().unwrap();
        assert!(calls.end(&call));
        let wanted = calls.begin(pages(0, 7..8));
        assert!(!wanted.cancelled_within(Duration::from_millis(5)));
    }

    #[test]
    fn a_wait_past_what_an_instant_holds_still_ends_on_a_cancel() {
        let calls = Arc::new(Calls::default());
        let call = calls.begin(Asked::Count { epoch: 0 });
        let model = Arc::clone(&calls);
        let cancel = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            model.drop_model();
        });
        assert!(call.cancelled_within(Duration::MAX));
        cancel.join().unwrap();
    }
}
