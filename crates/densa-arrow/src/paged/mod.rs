//! Rows of a table too large to hold, fetched a page at a time as the grid shows them.

mod source;
mod worker;

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::ops::Range;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;

use arrow_schema::SchemaRef;
use densa_grid::{CellStyle, CellText, Column, GridModel, Palette, SortOrder, CELL_BYTES};

pub use source::{FetchError, PageError, PageOptions, PageQuery, PageSource, SortKey};
use worker::{Ask, Fetched, Got, Wanted, Worker};

use crate::reader::{Format, Reader};
use crate::table::{
    columns_of, new_formats, size_column, widen_column, write_value, CellState, TableError,
};

/// A page held: its rows, its columns, and the epoch it was fetched in (a page of an earlier
/// one is shown until the page fetched again replaces it).
#[derive(Debug)]
struct Page {
    epoch: u64,
    rows: usize,
    readers: Vec<Reader>,
}

/// A grid model (for `densa_widgets::DataGrid`) of rows fetched a page at a time from a
/// [`PageSource`]: a table too large to hold, such as a query engine's result over files of
/// billions of rows. The grid shows only some rows, and the model fetches their pages and one
/// on each side, on a thread of its own, so the window never waits; rows not yet fetched show
/// empty until they arrive.
///
/// A click on a column title asks the source for its rows in that order. Cells are formatted
/// as in [`ArrowTable`](crate::ArrowTable); columns are sized by the first rows and widened
/// when later numbers no longer fit. Rows have no keys: a selection is a position. A copy of
/// selected rows holds those among them that are held ([`GridModel::copy_rows`]): selecting all
/// of a billion rows copies those held, which can be runs far apart. On a 32-bit target, rows
/// past `usize::MAX` are not shown.
///
/// Costs: a frame whose pages are held allocates nothing; one that asks for pages or takes
/// them in does. Dropping the model ends its thread once the source's current call returns.
#[derive(Debug)]
pub struct PagedModel {
    schema: SchemaRef,
    options: PageOptions,
    formats: Vec<Format>,
    /// Whether rows have sized the columns.
    sized: bool,
    columns: Vec<Column>,
    columns_revision: u64,
    /// Whether the source can sort by each column.
    sortable: Vec<bool>,
    pages: BTreeMap<u64, Page>,
    /// Pages asked for and not yet back.
    asked: BTreeSet<u64>,
    /// Pages the source failed to give, not asked again until a retry or a refresh.
    failed: BTreeSet<u64>,
    /// The rows there are, once counted.
    rows: Option<u64>,
    /// Whether the rows were counted since the last refresh: pages are asked for only then.
    counted: bool,
    /// Whether a count is on its way.
    counting: bool,
    /// The epoch the count on its way was asked in: one of an earlier epoch is dropped, one of
    /// a later epoch (a sort, which does not count again) taken.
    count_epoch: u64,
    query: PageQuery,
    sort: Option<(usize, SortOrder)>,
    /// Bumped by every new query or refresh: replies of earlier ones are dropped.
    epoch: u64,
    error: Option<FetchError>,
    /// Why the thread fetching pages stopped, once it has.
    stopped: Option<FetchError>,
    /// The pages wanted by the rows shown last.
    want: Range<u64>,
    wanted: Arc<Wanted>,
    asks: Sender<Ask>,
    gots: Receiver<Got>,
}

impl PagedModel {
    /// A model of the rows `source` gives, with `schema` (every page's columns), fetched on a
    /// thread of its own in pages of 256 rows.
    ///
    /// `wake` is called on that thread each time it sends something back (rows, a count, a
    /// failure), and possibly once after the model is dropped: give it
    /// `move || { waker.wake(); }` with a `densa_window::Waker`, so the window paints what
    /// came. If `wake` panics, the thread stops ([`FetchError::Stopped`]).
    ///
    /// # Errors
    /// The thread could not be started.
    pub fn new(
        schema: SchemaRef,
        source: impl PageSource,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        Self::with_options(schema, source, wake, PageOptions::default())
    }

    /// As [`new`](Self::new), paging as `options` say.
    ///
    /// # Errors
    /// The thread could not be started.
    pub fn with_options(
        schema: SchemaRef,
        source: impl PageSource,
        wake: impl Fn() + Send + 'static,
        options: PageOptions,
    ) -> io::Result<Self> {
        let options = PageOptions {
            page_rows: options.page_rows.max(1),
            pages_kept: options.pages_kept.max(1),
        };
        let sortable = schema
            .fields()
            .iter()
            .map(|field| source.can_sort(field.name()))
            .collect();
        let (asks, asked) = mpsc::channel();
        let (sent, gots) = mpsc::channel();
        let wanted = Arc::new(Wanted::default());
        let worker = Worker {
            source,
            schema: schema.clone(),
            page_rows: options.page_rows,
            asks: asked,
            gots: sent,
            wanted: Arc::clone(&wanted),
            wake: Box::new(wake),
        };
        thread::Builder::new()
            .name("densa-pages".to_owned())
            .spawn(move || worker.serve())?;
        let formats = new_formats(&schema);
        let columns = columns_of(&schema, &formats);
        let mut model = Self {
            schema,
            options,
            formats,
            sized: false,
            columns,
            columns_revision: 0,
            sortable,
            pages: BTreeMap::new(),
            asked: BTreeSet::new(),
            failed: BTreeSet::new(),
            rows: None,
            counted: false,
            counting: false,
            count_epoch: 0,
            query: PageQuery::new(),
            sort: None,
            epoch: 0,
            error: None,
            stopped: None,
            want: 0..0,
            wanted,
            asks,
            gots,
        };
        model.ask(true);
        Ok(model)
    }

    /// The schema of the rows.
    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// How many rows there are, once the source counted them: `None` until then, and after a
    /// count failed.
    pub fn row_count(&self) -> Option<u64> {
        self.rows
    }

    /// Why rows are missing: the source's last failure (to count, to give rows, rows that came
    /// back short), cleared by [`retry`](Self::retry), [`refresh`](Self::refresh) and sorting;
    /// or that the thread fetching pages stopped, which nothing clears.
    pub fn error(&self) -> Option<&FetchError> {
        self.stopped.as_ref().or(self.error.as_ref())
    }

    /// Whether `row`'s page is held as the source gives it now (not one kept from before a
    /// [`refresh`](Self::refresh) until it is fetched again).
    pub fn is_fetched(&self, row: usize) -> bool {
        self.pages
            .get(&self.page_of(row))
            .is_some_and(|page| page.epoch == self.epoch)
    }

    /// How many pages are held.
    pub fn pages_held(&self) -> usize {
        self.pages.len()
    }

    /// Rows a page holds.
    pub fn page_rows(&self) -> usize {
        self.options.page_rows
    }

    /// Asks again for what failed: the pages shown that the source failed to give, or the
    /// count (not one on its way). Pages held stay.
    pub fn retry(&mut self) {
        self.failed.clear();
        self.error = None;
        if !self.counted && !self.counting {
            self.ask(true);
        }
    }

    /// Counts the rows and fetches the pages shown again: for when the source's data changed.
    /// The rows held are shown until their pages are back, and the count until the new one is.
    /// Sorting fetches the pages of the new order by itself.
    pub fn refresh(&mut self) {
        self.ask(true);
    }

    /// Starts a new epoch: asks for the query from now on, counted again if `count`, and
    /// forgets what was asked or failed before.
    fn ask(&mut self, count: bool) {
        self.epoch = self.epoch.wrapping_add(1);
        self.asked.clear();
        self.failed.clear();
        self.error = None;
        if count {
            self.counted = false;
            self.counting = true;
            self.count_epoch = self.epoch;
        }
        // A thread that stopped is reported by `take_in`.
        let _ = self.asks.send(Ask::Query {
            epoch: self.epoch,
            query: self.query.clone(),
            count,
        });
    }

    /// As [`ArrowTable::set_decimals`](crate::ArrowTable::set_decimals): floating-point values
    /// of `column` with `decimals` fixed decimal places (`None`: the shortest exact form). A
    /// column sized by the pages is sized again from those held, as now printed; the grid
    /// takes in the new width.
    ///
    /// # Errors
    /// [`TableError::NoSuchColumn`] if there is no such column.
    pub fn set_decimals(
        &mut self,
        column: usize,
        decimals: Option<usize>,
    ) -> Result<(), TableError> {
        self.formats
            .get_mut(column)
            .ok_or(TableError::NoSuchColumn(column))?
            .decimals = decimals;
        self.size_again(column);
        Ok(())
    }

    /// As [`ArrowTable::set_column_chars`](crate::ArrowTable::set_column_chars): `column` is
    /// `chars` characters wide (1 to [`CELL_BYTES`]), longer text cut to fit ending in `…`, and
    /// pages no longer size it; the grid takes in the new width.
    ///
    /// # Errors
    /// [`TableError::NoSuchColumn`] if there is no such column.
    pub fn set_column_chars(&mut self, column: usize, chars: usize) -> Result<(), TableError> {
        let format = self
            .formats
            .get_mut(column)
            .ok_or(TableError::NoSuchColumn(column))?;
        format.max_chars = chars.clamp(1, CELL_BYTES);
        format.width_set = true;
        self.read_columns();
        Ok(())
    }

    /// Sizes `column` again from the pages held: by the first, widened by the others.
    fn size_again(&mut self, column: usize) {
        let (Some(field), Some(format)) = (
            self.schema.fields().get(column),
            self.formats.get_mut(column),
        ) else {
            return;
        };
        let mut held = self
            .pages
            .values()
            .filter_map(|page| page.readers.get(column));
        if let Some(first) = held.next() {
            size_column(format, first, field.name());
            let mut text = CellText::new();
            for reader in held {
                widen_column(format, reader, field.data_type(), &mut text);
            }
        }
        self.read_columns();
    }

    /// Reads the columns again and tells the grid.
    fn read_columns(&mut self) {
        self.columns = columns_of(&self.schema, &self.formats);
        self.columns_revision = self.columns_revision.wrapping_add(1);
    }

    /// The page holding `row`.
    fn page_of(&self, row: usize) -> u64 {
        // Lossless: usize is at most 64 bits wide.
        (row / self.options.page_rows) as u64
    }

    /// The page holding row `row` of the source's (a `u64`, counted by it).
    fn page_of_row(&self, row: u64) -> u64 {
        // Lossless: usize is at most 64 bits wide.
        row / self.options.page_rows as u64
    }

    /// The rows of `pages`, up to the last there is.
    fn rows_of(&self, pages: Range<u64>) -> Range<u64> {
        // Lossless: usize is at most 64 bits wide.
        let page_rows = self.options.page_rows as u64;
        let start = pages.start.saturating_mul(page_rows);
        let end = pages.end.saturating_mul(page_rows);
        start..self.rows.map_or(end, |rows| end.min(rows)).max(start)
    }

    /// Takes in what the thread sent back.
    fn take_in(&mut self) {
        loop {
            match self.gots.try_recv() {
                Ok(got) => self.take(got),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    // Said once: the thread says why it stopped when it can.
                    if self.stopped.is_none() {
                        self.stopped = Some(FetchError::Stopped {
                            message: "it ended without saying why".to_owned(),
                        });
                    }
                    return;
                }
            }
        }
    }

    /// Takes in one reply, unless it is of an epoch the model no longer wants.
    fn take(&mut self, got: Got) {
        match got {
            Got::Count { epoch, rows } if self.is_count(epoch) => {
                self.rows = Some(rows);
                self.counted = true;
                self.counting = false;
            }
            Got::Pages {
                epoch,
                first,
                pages,
            } if epoch == self.epoch => {
                for (fetched, page) in pages.into_iter().zip(first..) {
                    self.asked.remove(&page);
                    self.take_page(page, fetched);
                }
            }
            Got::Failed {
                epoch,
                pages: Some(pages),
                message,
            } if epoch == self.epoch => {
                for page in pages.clone() {
                    self.asked.remove(&page);
                    self.failed.insert(page);
                }
                let rows = self.rows_of(pages);
                self.error = Some(FetchError::Rows { rows, message });
            }
            Got::Failed {
                epoch,
                pages: None,
                message,
            } if self.is_count(epoch) => {
                // No rows to show: those counted before may be gone.
                self.rows = None;
                self.counting = false;
                self.error = Some(FetchError::Count { message });
            }
            Got::Skipped { epoch, pages } if epoch == self.epoch => {
                for page in pages {
                    self.asked.remove(&page);
                }
            }
            Got::Stopped { message } => self.stopped = Some(FetchError::Stopped { message }),
            // An epoch the model no longer wants.
            _ => {}
        }
    }

    /// Whether a count of `epoch` answers the count asked last: one of its epoch or later (a
    /// sort asks for none), not of one before (a refresh asks again). The thread answers the
    /// last count asked once (those asked before with it in one answer).
    fn is_count(&self, epoch: u64) -> bool {
        epoch.wrapping_sub(self.count_epoch) <= self.epoch.wrapping_sub(self.count_epoch)
    }

    /// Holds page `page` if it has every row the count says it has. One short of them (the data
    /// changed since it was counted) is not held, and says so.
    fn take_page(&mut self, page: u64, fetched: Fetched) {
        let rows = self.rows_of(page..page + 1);
        // Lossless: usize is at most 64 bits wide.
        let got = fetched.rows as u64;
        if got < rows.end - rows.start {
            self.failed.insert(page);
            self.error = Some(FetchError::Short { rows, got });
            return;
        }
        self.keep(page, fetched);
    }

    /// Holds `fetched` as page `page`: sizing the columns by the first rows, widening number and
    /// time columns for later ones, and dropping pages far from those wanted.
    fn keep(&mut self, page: u64, fetched: Fetched) {
        let readers: Vec<Reader> = fetched.columns.into_iter().map(Reader::new).collect();
        let fields = self.schema.fields();
        let mut text = CellText::new();
        let mut widened = false;
        for ((reader, format), field) in readers.iter().zip(&mut self.formats).zip(fields.iter()) {
            if self.sized {
                widened |= widen_column(format, reader, field.data_type(), &mut text);
            } else {
                size_column(format, reader, field.name());
            }
        }
        if widened || !self.sized {
            self.sized = true;
            self.read_columns();
        }
        let (epoch, rows) = (self.epoch, fetched.rows);
        self.pages.insert(
            page,
            Page {
                epoch,
                rows,
                readers,
            },
        );
        self.evict();
    }

    /// Drops pages past [`PageOptions::pages_kept`], those farthest from the pages wanted first;
    /// never a page wanted.
    fn evict(&mut self) {
        while self.pages.len() > self.options.pages_kept {
            let (Some(&low), Some(&high)) =
                (self.pages.keys().next(), self.pages.keys().next_back())
            else {
                return;
            };
            let below = self.want.start.saturating_sub(low);
            let above = high.saturating_sub(self.want.end.saturating_sub(1));
            let page = if below >= above { low } else { high };
            if self.want.contains(&page) {
                // Every page held is wanted.
                return;
            }
            self.pages.remove(&page);
        }
    }

    /// Asks for the pages of `rows` and one on each side, those not held as the source gives
    /// them now, asked or failed; none until the rows are counted.
    fn ask_pages(&mut self, rows: Range<usize>) {
        let Some(count) = self.rows.filter(|&n| n > 0 && self.counted) else {
            return;
        };
        if rows.is_empty() {
            return;
        }
        let first = self.page_of(rows.start).saturating_sub(1);
        let last = (self.page_of(rows.end - 1) + 1).min(self.page_of_row(count - 1));
        if first > last {
            return;
        }
        self.want = first..last + 1;
        self.wanted.set(first, last);
        // Allocates only when there are pages to ask for.
        let mut pages = Vec::new();
        for page in first..=last {
            let known = self.pages.get(&page).is_some_and(|p| p.epoch == self.epoch)
                || self.asked.contains(&page)
                || self.failed.contains(&page);
            if !known {
                pages.push(page);
            }
        }
        if pages.is_empty() {
            return;
        }
        self.asked.extend(pages.iter().copied());
        // A thread that stopped is reported by `take_in`.
        let _ = self.asks.send(Ask::Pages {
            epoch: self.epoch,
            pages,
        });
    }

    /// Writes the cell at `row`, `column` if its page is held.
    fn write(&self, row: usize, column: usize, text: &mut CellText) -> CellState {
        let local = row % self.options.page_rows;
        let reader = self
            .pages
            .get(&self.page_of(row))
            .and_then(|page| page.readers.get(column));
        match (reader, self.formats.get(column)) {
            (Some(reader), Some(format)) if local < reader.len() => {
                write_value(reader, local, format, text)
            }
            _ => CellState::OutOfRange,
        }
    }
}

impl GridModel for PagedModel {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn rows(&self) -> usize {
        self.rows
            .map_or(0, |rows| usize::try_from(rows).unwrap_or(usize::MAX))
    }

    fn cell(&self, row: usize, column: usize, text: &mut CellText, palette: &Palette) -> CellStyle {
        let color = match self.write(row, column, text) {
            CellState::Value => palette.text,
            _ => palette.dim_text,
        };
        CellStyle::new(color)
    }

    fn prepare(&mut self) {
        self.take_in();
    }

    fn showing(&mut self, rows: Range<usize>) {
        self.ask_pages(rows);
    }

    /// The rows of `rows` held, in runs: a copy holds no rows not fetched.
    fn copy_rows(&self, rows: Range<usize>) -> Vec<Range<usize>> {
        let mut runs: Vec<Range<usize>> = Vec::new();
        if rows.is_empty() {
            return runs;
        }
        let pages = self.page_of(rows.start)..=self.page_of(rows.end - 1);
        for (&page, held) in self.pages.range(pages) {
            let Some(start) = usize::try_from(page)
                .ok()
                .and_then(|page| page.checked_mul(self.options.page_rows))
            else {
                continue;
            };
            let run = start.max(rows.start)..start.saturating_add(held.rows).min(rows.end);
            if run.is_empty() {
                continue;
            }
            match runs.last_mut() {
                Some(last) if last.end == run.start => last.end = run.end,
                _ => runs.push(run),
            }
        }
        runs
    }

    /// Asks the source for its rows sorted by `column`; `false` for a column the schema does
    /// not have, whose name an earlier column has (the source sorts by name), or that the
    /// source cannot sort by ([`PageSource::can_sort`]).
    fn sort(&mut self, column: usize, order: SortOrder) -> bool {
        let Some(field) = self.schema.fields().get(column) else {
            return false;
        };
        let named_once = self
            .schema
            .column_with_name(field.name())
            .is_some_and(|(at, _)| at == column);
        if !named_once || !self.sortable.get(column).copied().unwrap_or(false) {
            return false;
        }
        self.sort = Some((column, order));
        self.query = PageQuery::new().sorted_by(SortKey::new(field.name().clone(), order));
        // The rows held are in the old order. A new order has as many rows: counted only if
        // they were not, and no count is on its way.
        self.pages.clear();
        self.ask(!self.counted && !self.counting);
        true
    }

    fn sort_state(&self) -> Option<(usize, SortOrder)> {
        self.sort
    }

    fn columns_revision(&self) -> u64 {
        self.columns_revision
    }
}
