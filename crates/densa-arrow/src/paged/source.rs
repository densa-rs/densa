//! What a paged model reads (a source of rows, a page at a time, in the order asked for), how
//! it pages, and why rows can be missing.

use std::error::Error;
use std::fmt;
use std::ops::Range;

use arrow_array::RecordBatch;
use densa_grid::SortOrder;

/// What a [`PageSource`] fails with: any error, sent across threads.
pub type PageError = Box<dyn Error + Send + Sync>;

/// Rows a page holds unless told otherwise: several screens.
const PAGE_ROWS: usize = 256;
/// Pages kept unless told otherwise.
const PAGES_KEPT: usize = 64;

/// A column rows are sorted by, and which way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    column: String,
    order: SortOrder,
}

impl SortKey {
    /// Rows by `column` (its name) in `order`.
    pub fn new(column: impl Into<String>, order: SortOrder) -> Self {
        Self {
            column: column.into(),
            order,
        }
    }

    /// The column's name.
    pub fn column(&self) -> &str {
        &self.column
    }

    /// Which way.
    pub fn order(&self) -> SortOrder {
        self.order
    }

    /// Whether the largest come first.
    pub fn is_descending(&self) -> bool {
        self.order == SortOrder::Descending
    }
}

/// What a [`PageSource`] is asked for: its rows, in an order (an `ORDER BY`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageQuery {
    sort: Vec<SortKey>,
}

impl PageQuery {
    /// The source's rows in its own order.
    pub fn new() -> Self {
        Self::default()
    }

    /// The query with its rows sorted by `key` too, after the keys it has.
    #[must_use]
    pub fn sorted_by(mut self, key: SortKey) -> Self {
        self.sort.push(key);
        self
    }

    /// The keys rows are sorted by, the first first; none for the source's own order.
    pub fn sort(&self) -> &[SortKey] {
        &self.sort
    }
}

/// The rows of a table too large to hold, a page at a time: what a
/// [`PagedModel`](super::PagedModel) reads. Its calls (but [`can_sort`](Self::can_sort)) run on
/// a thread of the model's own, so a call may take as long as it needs: a query to a database
/// or a query engine. A call is not interrupted; the next waits for it. Rows are numbered from
/// 0 in the order the query gives them.
///
/// A call that fails, or panics (with panics unwinding, the default), is reported by the model
/// ([`PagedModel::error`](super::PagedModel::error)), and the next calls go on.
pub trait PageSource: Send + 'static {
    /// How many rows `query` gives.
    ///
    /// # Errors
    /// Any the source meets: the model shows no rows ([`FetchError::Count`]) until
    /// [`retry`](super::PagedModel::retry) or [`refresh`](super::PagedModel::refresh) counts
    /// them again.
    fn row_count(&mut self, query: &PageQuery) -> Result<u64, PageError>;

    /// `limit` rows from row `offset` of what `query` gives (fewer only at the end), in as many
    /// batches as they come in, each with the model's columns: their data types, in order (names
    /// are not compared). The model asks for pages side by side in one call; when such a call
    /// fails, it asks for each of its pages alone.
    ///
    /// # Errors
    /// Any the source meets: those rows stay empty ([`FetchError::Rows`]) until
    /// [`retry`](super::PagedModel::retry) or [`refresh`](super::PagedModel::refresh). Fewer
    /// rows than asked for before the end say the data changed since it was counted
    /// ([`FetchError::Short`]).
    fn page(
        &mut self,
        query: &PageQuery,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, PageError>;

    /// Whether the source can give its rows sorted by `column`: a click on its title sorts only
    /// then. Asked once for each column when the model is made, on the thread making it; every
    /// column can by default.
    fn can_sort(&self, column: &str) -> bool {
        let _ = column;
        true
    }
}

/// How a [`PagedModel`](super::PagedModel) pages: the rows of a page and how many pages it
/// keeps.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PageOptions {
    /// Rows a page holds: 256 unless set, at least 1.
    pub page_rows: usize,
    /// Pages kept, 64 unless set, at least 1: those farthest from the rows shown go first
    /// (fetched again if shown again). Pages of the rows shown are kept even past it.
    pub pages_kept: usize,
}

impl Default for PageOptions {
    fn default() -> Self {
        Self {
            page_rows: PAGE_ROWS,
            pages_kept: PAGES_KEPT,
        }
    }
}

impl PageOptions {
    /// The options with pages of `rows` rows.
    #[must_use]
    pub fn page_rows(mut self, rows: usize) -> Self {
        self.page_rows = rows;
        self
    }

    /// The options keeping `pages` pages.
    #[must_use]
    pub fn pages_kept(mut self, pages: usize) -> Self {
        self.pages_kept = pages;
        self
    }
}

/// Why a [`PagedModel`](super::PagedModel) has rows it cannot show: what
/// [`PagedModel::error`](super::PagedModel::error) gives.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FetchError {
    /// The source failed to count the rows (`message` says why): no rows are shown until
    /// [`retry`](super::PagedModel::retry) or [`refresh`](super::PagedModel::refresh) counts
    /// them.
    Count {
        /// The source's error, or what its panic said.
        message: String,
    },
    /// The source failed to give `rows` (`message` says why): they stay empty until
    /// [`retry`](super::PagedModel::retry) or [`refresh`](super::PagedModel::refresh).
    Rows {
        /// The rows asked for, those of one page.
        rows: Range<u64>,
        /// The source's error, or what its panic said.
        message: String,
    },
    /// The source gave only `got` of `rows`, which its count says are there: the data changed
    /// since it was counted. They stay empty until [`refresh`](super::PagedModel::refresh)
    /// counts again.
    Short {
        /// The rows asked for, those of one page.
        rows: Range<u64>,
        /// How many came.
        got: u64,
    },
    /// The thread fetching pages stopped (`message` says why: the model's `wake` panicked):
    /// nothing more arrives, so make the model again.
    Stopped {
        /// What stopped it.
        message: String,
    },
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Count { message } => write!(f, "the rows could not be counted: {message}"),
            Self::Rows { rows, message } => write!(
                f,
                "rows {} to {} could not be read: {message}",
                rows.start,
                rows.end.saturating_sub(1)
            ),
            Self::Short { rows, got } => write!(
                f,
                "rows {} to {} came back short ({got} of {}): the data changed since it was \
                 counted; refresh to count it again",
                rows.start,
                rows.end.saturating_sub(1),
                rows.end.saturating_sub(rows.start)
            ),
            Self::Stopped { message } => write!(
                f,
                "the thread fetching pages stopped ({message}): make the model again"
            ),
        }
    }
}

impl Error for FetchError {}
