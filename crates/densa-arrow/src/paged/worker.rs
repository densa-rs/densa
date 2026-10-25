//! The thread fetching pages: it counts and fetches, the pages wanted nearest first, those side
//! by side in one call (each alone once such a call failed).

use std::any::Any;
use std::collections::BTreeSet;
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;

use arrow_array::{new_empty_array, Array, ArrayRef, RecordBatch};
use arrow_schema::SchemaRef;

use super::source::{PageError, PageQuery, PageSource};

/// Pages wanted side by side, fetched in one call at most.
const PAGES_PER_CALL: u64 = 4;

/// What the model asks its thread for.
#[derive(Debug)]
pub(super) enum Ask {
    /// The rows of `query` from now on, as epoch `epoch`, counted again if `count`.
    Query {
        epoch: u64,
        query: PageQuery,
        count: bool,
    },
    /// Pages `pages` of epoch `epoch`: all those a frame wanted, so the thread fetches those
    /// side by side in one call.
    Pages { epoch: u64, pages: Vec<u64> },
}

/// What the thread sends back, of epoch `epoch` (the model drops those it no longer wants).
#[derive(Debug)]
pub(super) enum Got {
    Count {
        epoch: u64,
        rows: u64,
    },
    /// Pages `first`, `first + 1`, ..., in order.
    Pages {
        epoch: u64,
        first: u64,
        pages: Vec<Fetched>,
    },
    /// `pages` (or the count, for `None`) could not be read: `message` says why.
    Failed {
        epoch: u64,
        pages: Option<Range<u64>>,
        message: String,
    },
    /// `pages` were not fetched: when their turn came, they were not among the pages wanted.
    Skipped {
        epoch: u64,
        pages: Vec<u64>,
    },
    /// The thread stopped: `message` says why. Nothing follows.
    Stopped {
        message: String,
    },
}

/// A page fetched: its rows, and its columns, each that long.
#[derive(Debug)]
pub(super) struct Fetched {
    pub(super) rows: usize,
    pub(super) columns: Vec<ArrayRef>,
}

/// The pages the model wants now, first and last, for its thread to fetch the nearest first and
/// to skip those no longer wanted.
#[derive(Debug, Default)]
pub(super) struct Wanted {
    first: AtomicU64,
    last: AtomicU64,
}

impl Wanted {
    /// Says pages `first..=last` are wanted.
    pub(super) fn set(&self, first: u64, last: u64) {
        // Relaxed: hints of which pages to fetch first and which to skip, stored before the
        // asks that follow them through a channel (which orders them), so the thread reads them
        // at least that new. A pair read torn (one from another frame) fetches a page early or
        // skips one, which is asked for again.
        self.first.store(first, Ordering::Relaxed);
        self.last.store(last, Ordering::Relaxed);
    }

    fn get(&self) -> (u64, u64) {
        // Relaxed: see `set`.
        (
            self.first.load(Ordering::Relaxed),
            self.last.load(Ordering::Relaxed),
        )
    }
}

/// The pages asked for and not yet fetched.
#[derive(Debug, Default)]
struct Queue {
    pages: BTreeSet<u64>,
    /// Of those, the pages of a call that failed: each is fetched alone.
    alone: BTreeSet<u64>,
}

impl Queue {
    fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    fn clear(&mut self) {
        self.pages.clear();
        self.alone.clear();
    }

    /// Puts back `pages`, a call's that failed, each to be fetched alone.
    fn put_back(&mut self, pages: Range<u64>) {
        self.pages.extend(pages.clone());
        self.alone.extend(pages);
    }

    /// Drops the pages outside `first..=last` and gives them.
    fn skip(&mut self, first: u64, last: u64) -> Vec<u64> {
        let mut inside = self.pages.split_off(&first);
        let above = match last.checked_add(1) {
            Some(after) => inside.split_off(&after),
            None => BTreeSet::new(),
        };
        let below = std::mem::replace(&mut self.pages, inside);
        let skipped: Vec<u64> = below.into_iter().chain(above).collect();
        for page in &skipped {
            self.alone.remove(page);
        }
        skipped
    }

    /// The page nearest the middle of `first..=last`, with those beside it up to
    /// [`PAGES_PER_CALL`] (none with a page fetched alone): one call's pages, taken from the
    /// queue.
    fn next(&mut self, first: u64, last: u64) -> Option<Range<u64>> {
        let middle = first.midpoint(last);
        let above = self.pages.range(middle..).next().copied();
        let below = self.pages.range(..middle).next_back().copied();
        let nearest = match (below, above) {
            (Some(below), Some(above)) if middle - below <= above - middle => below,
            (_, Some(above)) => above,
            (Some(below), None) => below,
            (None, None) => return None,
        };
        let (mut start, mut end) = (nearest, nearest + 1);
        if !self.alone.contains(&nearest) {
            let joins = |page: u64| self.pages.contains(&page) && !self.alone.contains(&page);
            while end - start < PAGES_PER_CALL && joins(end) {
                end += 1;
            }
            while end - start < PAGES_PER_CALL && start > 0 && joins(start - 1) {
                start -= 1;
            }
        }
        for page in start..end {
            self.pages.remove(&page);
            self.alone.remove(&page);
        }
        Some(start..end)
    }
}

/// The thread fetching pages.
pub(super) struct Worker<S> {
    pub(super) source: S,
    pub(super) schema: SchemaRef,
    pub(super) page_rows: usize,
    pub(super) asks: Receiver<Ask>,
    pub(super) gots: Sender<Got>,
    pub(super) wanted: Arc<Wanted>,
    pub(super) wake: Box<dyn Fn() + Send>,
}

impl<S: PageSource> Worker<S> {
    /// Fetches until the model is dropped. If anything panics on the way (the source's panics
    /// are failures of their calls, so the model's `wake`), says why the thread stops.
    pub(super) fn serve(mut self) {
        let Err(panic) = panic::catch_unwind(AssertUnwindSafe(|| self.run())) else {
            return;
        };
        let message = format!("a panic: {}", panic_text(&*panic));
        if self.gots.send(Got::Stopped { message }).is_ok() {
            // Once more, so the model hears of it soon: a wake that panicked may panic again.
            let _ = panic::catch_unwind(AssertUnwindSafe(|| (self.wake)()));
        }
    }

    fn run(&mut self) {
        let mut epoch = 0;
        let mut query = PageQuery::new();
        let mut queue = Queue::default();
        loop {
            // Waits for news only with nothing to do.
            let mut count = false;
            if queue.is_empty() {
                match self.asks.recv() {
                    Ok(ask) => count |= take(ask, &mut epoch, &mut query, &mut queue),
                    Err(_) => return,
                }
            }
            // Everything asked since: the newest query is the one that counts.
            loop {
                match self.asks.try_recv() {
                    Ok(ask) => count |= take(ask, &mut epoch, &mut query, &mut queue),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if count {
                self.count(epoch, &query);
            }
            let (first, last) = self.wanted.get();
            // Read torn, the pages wanted can come out backwards: then nothing is skipped
            // rather than pages still wanted.
            if first <= last {
                let pages = queue.skip(first, last);
                if !pages.is_empty() {
                    self.send(Got::Skipped { epoch, pages });
                }
            }
            if let Some(pages) = queue.next(first, last) {
                self.fetch(epoch, &query, pages, &mut queue);
            }
        }
    }

    /// Calls the source, a panic becoming a failure.
    fn call<T>(&mut self, call: impl FnOnce(&mut S) -> Result<T, PageError>) -> Result<T, String> {
        match panic::catch_unwind(AssertUnwindSafe(|| call(&mut self.source))) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(error.to_string()),
            Err(panic) => Err(format!("the page source panicked: {}", panic_text(&*panic))),
        }
    }

    fn count(&mut self, epoch: u64, query: &PageQuery) {
        let got = match self.call(|source| source.row_count(query)) {
            Ok(rows) => Got::Count { epoch, rows },
            Err(message) => Got::Failed {
                epoch,
                pages: None,
                message,
            },
        };
        self.send(got);
    }

    /// Fetches `pages` in one call and sends them back one page each. A call of several pages
    /// that fails puts them back to be fetched alone, so a page the source cannot give does not
    /// take those beside it along.
    fn fetch(&mut self, epoch: u64, query: &PageQuery, pages: Range<u64>, queue: &mut Queue) {
        // Lossless: usize is at most 64 bits wide; a call is a few pages.
        let (page_rows, count) = (self.page_rows as u64, (pages.end - pages.start) as usize);
        let offset = pages.start.saturating_mul(page_rows);
        let limit = self.page_rows.saturating_mul(count);
        let fetched = self
            .call(|source| source.page(query, offset, limit))
            .and_then(|batches| self.join(&batches));
        let got = match fetched {
            Ok((columns, rows)) => Got::Pages {
                epoch,
                first: pages.start,
                pages: split(&columns, rows, self.page_rows, count),
            },
            Err(_) if count > 1 => {
                queue.put_back(pages);
                return;
            }
            Err(message) => Got::Failed {
                epoch,
                pages: Some(pages),
                message,
            },
        };
        self.send(got);
    }

    /// The columns of `batches` joined, and their rows; an error naming what differs from the
    /// model's columns.
    fn join(&self, batches: &[RecordBatch]) -> Result<(Vec<ArrayRef>, usize), String> {
        for batch in batches {
            self.check(batch)?;
        }
        let rows = batches.iter().map(RecordBatch::num_rows).sum();
        let columns = self
            .schema
            .fields()
            .iter()
            .enumerate()
            .map(|(at, field)| match batches {
                [] => Ok(new_empty_array(field.data_type())),
                [one] => Ok(Arc::clone(one.column(at))),
                many => {
                    let arrays: Vec<&dyn Array> =
                        many.iter().map(|batch| batch.column(at).as_ref()).collect();
                    arrow_select::concat::concat(&arrays).map_err(|error| {
                        format!("the page source's batches could not be joined: {error}")
                    })
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((columns, rows))
    }

    /// Whether `batch` has the model's columns: their data types, in order.
    fn check(&self, batch: &RecordBatch) -> Result<(), String> {
        let ours = self.schema.fields();
        let theirs = batch.schema_ref().fields();
        if ours.len() != theirs.len() {
            return Err(format!(
                "the page source gave {} columns, the model's schema has {}: give pages with \
                 the schema the model was made with",
                theirs.len(),
                ours.len()
            ));
        }
        match ours
            .iter()
            .zip(theirs.iter())
            .find(|(a, b)| a.data_type() != b.data_type())
        {
            Some((a, b)) => Err(format!(
                "the page source gave column {} as {}, the model's schema has {}",
                a.name(),
                b.data_type(),
                a.data_type()
            )),
            None => Ok(()),
        }
    }

    fn send(&self, got: Got) {
        // A model dropped meanwhile: the next wait for news ends the thread.
        if self.gots.send(got).is_ok() {
            (self.wake)();
        }
    }
}

/// Takes in `ask`: a new query replaces the old one and the pages asked of it; a page of the
/// query now is queued. Whether the rows need counting.
fn take(ask: Ask, epoch: &mut u64, query: &mut PageQuery, queue: &mut Queue) -> bool {
    match ask {
        Ask::Query {
            epoch: e,
            query: q,
            count,
        } => {
            *epoch = e;
            *query = q;
            queue.clear();
            count
        }
        Ask::Pages { epoch: e, pages } => {
            // Asks come in order: one of an earlier epoch came before the query that ended it,
            // which cleared it. This only guards that order.
            if e == *epoch {
                queue.pages.extend(pages);
            }
            false
        }
    }
}

/// `columns` (`rows` long) cut into `pages` pages of `page_rows`: those past the rows are
/// short or empty, rows past the pages left out.
fn split(columns: &[ArrayRef], rows: usize, page_rows: usize, pages: usize) -> Vec<Fetched> {
    (0..pages)
        .map(|page| {
            let start = page.saturating_mul(page_rows).min(rows);
            let len = page_rows.min(rows - start);
            Fetched {
                rows: len,
                columns: columns
                    .iter()
                    .map(|column| column.slice(start, len))
                    .collect(),
            }
        })
        .collect()
}

/// What a panic said, if it said it as text.
fn panic_text(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message")
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;
    use std::time::Duration;

    use arrow_array::{ArrayRef, Int64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};

    use super::*;

    #[test]
    fn the_queue_gives_the_middle_first_then_the_nearer_side() {
        let mut queue = Queue::default();
        queue.pages.extend(0..10);
        assert_eq!(queue.next(0, 9), Some(4..8));
        assert_eq!(queue.next(0, 9), Some(0..4));
        assert_eq!(queue.next(0, 9), Some(8..10));
        assert_eq!(queue.next(0, 9), None);
    }

    #[test]
    fn pages_put_back_are_fetched_alone_and_join_no_call() {
        let mut queue = Queue::default();
        queue.pages.extend([4, 6]);
        queue.put_back(5..6);
        // The middle, alone though pages are queued on both sides of it.
        assert_eq!(queue.next(4, 6), Some(5..6));
        assert_eq!(queue.next(4, 6), Some(4..5));
        assert_eq!(queue.next(4, 6), Some(6..7));

        queue.pages.extend([6, 7]);
        queue.put_back(4..6);
        // 6, the middle, with 7 after it but not 5 before it: 5 and 4 go alone.
        assert_eq!(queue.next(4, 8), Some(6..8));
        assert_eq!(queue.next(4, 8), Some(5..6));
        assert_eq!(queue.next(4, 8), Some(4..5));
        assert!(queue.is_empty() && queue.alone.is_empty());
    }

    #[test]
    fn a_page_skipped_is_no_longer_fetched_alone() {
        let mut queue = Queue::default();
        queue.put_back(4..8);
        assert_eq!(queue.skip(5, 6), [4, 7]);
        assert_eq!(queue.alone, BTreeSet::from([5, 6]));
        queue.clear();
        assert!(queue.is_empty() && queue.alone.is_empty());
    }

    /// One page of `n`, the row number, for any offset.
    struct Rows;

    impl PageSource for Rows {
        fn row_count(&mut self, _: &PageQuery) -> Result<u64, PageError> {
            Ok(1_000)
        }

        fn page(
            &mut self,
            _: &PageQuery,
            offset: u64,
            limit: usize,
        ) -> Result<Vec<RecordBatch>, PageError> {
            let start = i64::try_from(offset)?;
            let n: Vec<i64> = (start..start + i64::try_from(limit)?).collect();
            let n: ArrayRef = Arc::new(Int64Array::from(n));
            Ok(vec![RecordBatch::try_from_iter([("n", n)])?])
        }
    }

    /// A worker over [`Rows`] in pages of 10 on a thread: its asks, its replies, its hints.
    fn worker() -> (Sender<Ask>, Receiver<Got>, Arc<Wanted>) {
        let (asks, asked) = mpsc::channel();
        let (sent, gots) = mpsc::channel();
        let wanted = Arc::new(Wanted::default());
        let worker = Worker {
            source: Rows,
            schema: Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)])),
            page_rows: 10,
            asks: asked,
            gots: sent,
            wanted: Arc::clone(&wanted),
            wake: Box::new(|| {}),
        };
        thread::spawn(move || worker.serve());
        (asks, gots, wanted)
    }

    #[test]
    fn hints_read_backwards_skip_nothing() {
        let (asks, gots, wanted) = worker();
        let query = PageQuery::new();
        let (epoch, count) = (1, false);
        asks.send(Ask::Query {
            epoch,
            query,
            count,
        })
        .unwrap();
        // As if read torn: the first page wanted from one frame, the last from another.
        wanted.set(1_000, 10);
        asks.send(Ask::Pages {
            epoch,
            pages: vec![5],
        })
        .unwrap();
        let got = gots.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            matches!(got, Got::Pages { first: 5, .. }),
            "fetched, not skipped: {got:?}"
        );
    }
}
