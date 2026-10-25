//! A source of numbered rows that records, gates and fails its calls, and helpers that
//! wait for the model's thread or paint frames as the grid does.

use std::collections::{BTreeSet, VecDeque};
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use densa_arrow::{PageError, PageOptions, PageQuery, PageSource, PagedModel, SortKey};
use densa_grid::{CellText, GridModel, Palette, SortOrder};

pub(crate) type Sort = Option<(String, SortOrder)>;

/// What the source was asked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Call {
    Count(Sort),
    Page {
        offset: u64,
        limit: usize,
        sort: Sort,
    },
}

/// Rows `0..rows`: `n`, the row, `name`, `row n`, and `third`, a third of `n`; any column
/// sorts them, from the last when descending. It records each call as it starts; with a
/// gate, each call then waits for a go.
pub(crate) struct Numbers {
    rows: u64,
    calls: Arc<Mutex<Vec<Call>>>,
    gate: Option<Receiver<()>>,
    /// Offsets of calls that fail, once each.
    pub(crate) failing: BTreeSet<u64>,
    /// Offsets of calls that panic, once each.
    pub(crate) panicking: BTreeSet<u64>,
    /// Rows it cannot read: every call reaching into them fails.
    pub(crate) unreadable: Option<Range<u64>>,
    /// Rows a call gives at most.
    pub(crate) most: Option<u64>,
    /// Count calls that fail, by their number (from 0).
    pub(crate) failing_counts: BTreeSet<u32>,
    /// Count calls so far.
    count_calls: u32,
    /// Gives `n` as text: not the model's schema.
    pub(crate) wrong: bool,
    /// Gives `n` alone: fewer columns than the model's.
    pub(crate) narrow: bool,
    /// Gives each page as two batches.
    pub(crate) halves: bool,
    /// Columns it cannot sort by.
    pub(crate) unsortable: Vec<&'static str>,
    /// Rows the data has from each count call on, one per call (then `rows` stays).
    pub(crate) counts: VecDeque<u64>,
    /// Told when the source is dropped.
    pub(crate) dropped: Option<Sender<()>>,
}

impl Numbers {
    pub(crate) fn new(rows: u64) -> (Self, Arc<Mutex<Vec<Call>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let source = Self {
            rows,
            calls: Arc::clone(&calls),
            gate: None,
            failing: BTreeSet::new(),
            panicking: BTreeSet::new(),
            unreadable: None,
            most: None,
            failing_counts: BTreeSet::new(),
            count_calls: 0,
            wrong: false,
            narrow: false,
            halves: false,
            unsortable: Vec::new(),
            counts: VecDeque::new(),
            dropped: None,
        };
        (source, calls)
    }

    /// The source with a gate: each call waits for a send on the sender returned.
    pub(crate) fn gated(mut self) -> (Self, Sender<()>) {
        let (go, gate) = mpsc::channel();
        self.gate = Some(gate);
        (self, go)
    }

    fn wait(&self) {
        if let Some(gate) = &self.gate {
            let _ = gate.recv();
        }
    }

    /// The batch of rows `range` in `query`'s order.
    fn rows_of(&self, query: &PageQuery, range: Range<u64>) -> RecordBatch {
        let descending = query.sort().first().is_some_and(SortKey::is_descending);
        let ns: Vec<i64> = range
            .map(|i| if descending { self.rows - 1 - i } else { i })
            .map(|n| i64::try_from(n).unwrap())
            .collect();
        let names: Vec<String> = ns.iter().map(|n| format!("row {n}")).collect();
        let thirds: Vec<f64> = ns.iter().map(|&n| n as f64 / 3.0).collect();
        let n: ArrayRef = if self.wrong {
            Arc::new(StringArray::from(names.clone()))
        } else {
            Arc::new(Int64Array::from(ns))
        };
        if self.narrow {
            return RecordBatch::try_from_iter([("n", n)]).unwrap();
        }
        RecordBatch::try_from_iter([
            ("n", n),
            ("name", Arc::new(StringArray::from(names)) as ArrayRef),
            ("third", Arc::new(Float64Array::from(thirds)) as ArrayRef),
        ])
        .unwrap()
    }
}

fn sort_of(query: &PageQuery) -> Sort {
    query
        .sort()
        .first()
        .map(|key| (key.column().to_owned(), key.order()))
}

impl PageSource for Numbers {
    fn row_count(&mut self, query: &PageQuery) -> Result<u64, PageError> {
        self.calls.lock().unwrap().push(Call::Count(sort_of(query)));
        self.wait();
        self.count_calls += 1;
        if self.failing_counts.contains(&(self.count_calls - 1)) {
            return Err("the count is off today".into());
        }
        if let Some(rows) = self.counts.pop_front() {
            self.rows = rows;
        }
        Ok(self.rows)
    }

    fn page(
        &mut self,
        query: &PageQuery,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, PageError> {
        self.calls.lock().unwrap().push(Call::Page {
            offset,
            limit,
            sort: sort_of(query),
        });
        self.wait();
        if self.panicking.remove(&offset) {
            panic!("page {offset} blew up");
        }
        if self.failing.remove(&offset) {
            return Err("the disk is on fire".into());
        }
        if offset >= self.rows {
            return Err(format!("offset {offset} is past the end").into());
        }
        let end = offset + limit as u64;
        if let Some(rows) = self
            .unreadable
            .clone()
            .filter(|rows| rows.start < end && offset < rows.end)
        {
            return Err(format!("rows {} to {} are unreadable", rows.start, rows.end - 1).into());
        }
        let end = end
            .min(self.rows)
            .min(offset + self.most.unwrap_or(u64::MAX - offset));
        if self.halves && end > offset + 1 {
            let middle = offset + (end - offset) / 2;
            return Ok(vec![
                self.rows_of(query, offset..middle),
                self.rows_of(query, middle..end),
            ]);
        }
        Ok(vec![self.rows_of(query, offset..end)])
    }

    fn can_sort(&self, column: &str) -> bool {
        !self.unsortable.contains(&column)
    }
}

impl Drop for Numbers {
    fn drop(&mut self) {
        if let Some(dropped) = &self.dropped {
            let _ = dropped.send(());
        }
    }
}

pub(crate) fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("n", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("third", DataType::Float64, false),
    ]))
}

/// A model of `source` in pages of `page_rows`.
pub(crate) fn model(source: Numbers, page_rows: usize) -> PagedModel {
    let options = PageOptions::default().page_rows(page_rows);
    PagedModel::with_options(schema(), source, || {}, options).unwrap()
}

/// A wake that counts its calls, and the count.
pub(crate) fn counted_wake() -> (impl Fn() + Send + 'static, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let woken = Arc::clone(&wakes);
    let wake = move || {
        woken.fetch_add(1, Ordering::Relaxed);
    };
    (wake, wakes)
}

/// Waits (5 s at most) until `wakes` reaches `n`: until that many replies were sent.
pub(crate) fn until_woken(wakes: &AtomicUsize, n: usize) {
    let start = Instant::now();
    while wakes.load(Ordering::Relaxed) < n {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "never woken {n} times"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Takes in what arrived until `done` holds (5 s at most).
pub(crate) fn until(model: &mut PagedModel, done: impl Fn(&PagedModel) -> bool) {
    let start = Instant::now();
    while !done(model) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "timed out: {:?} rows, {} pages, error {:?}",
            model.row_count(),
            model.pages_held(),
            model.error()
        );
        model.prepare();
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Paints frames as the grid does (takes in what arrived, then says which rows it shows) until
/// `done` holds (5 s at most).
pub(crate) fn frames(
    model: &mut PagedModel,
    rows: Range<usize>,
    done: impl Fn(&PagedModel) -> bool,
) {
    let start = Instant::now();
    while !done(model) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "timed out: {:?} rows, {} pages, error {:?}",
            model.row_count(),
            model.pages_held(),
            model.error()
        );
        model.prepare();
        model.showing(rows.clone());
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The text of a cell, and whether it is in the text color (not dimmed).
pub(crate) fn cell(model: &PagedModel, row: usize, column: usize) -> (String, bool) {
    let palette = Palette::new();
    let mut text = CellText::new();
    let style = model.cell(row, column, &mut text, &palette);
    (text.as_str().to_owned(), style.color == palette.text)
}

/// What the model's error says.
pub(crate) fn error(model: &PagedModel) -> String {
    model.error().map(ToString::to_string).unwrap_or_default()
}

/// The pages the source was asked for, call by call, for pages of `page_rows`.
pub(crate) fn pages_asked(calls: &Mutex<Vec<Call>>, page_rows: u64) -> Vec<u64> {
    calls
        .lock()
        .unwrap()
        .iter()
        .flat_map(|call| match call {
            Call::Page { offset, limit, .. } => {
                let first = offset / page_rows;
                first..first + *limit as u64 / page_rows
            }
            Call::Count(_) => 0..0,
        })
        .collect()
}

/// The page calls made, as (offset, limit).
pub(crate) fn page_calls(calls: &Mutex<Vec<Call>>) -> Vec<(u64, usize)> {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|call| match call {
            Call::Page { offset, limit, .. } => Some((*offset, *limit)),
            Call::Count(_) => None,
        })
        .collect()
}

/// The counts made.
pub(crate) fn counts(calls: &Mutex<Vec<Call>>) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| matches!(call, Call::Count(_)))
        .count()
}

/// Waits (5 s at most) until the source was asked to count `n` times.
pub(crate) fn until_counting(calls: &Mutex<Vec<Call>>, n: usize) {
    let start = Instant::now();
    while counts(calls) < n {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "never counted {n} times"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Waits (5 s at most) until the source was asked for pages `n` times.
pub(crate) fn until_paging(calls: &Mutex<Vec<Call>>, n: usize) {
    let start = Instant::now();
    while page_calls(calls).len() < n {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "never paged {n} times"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
