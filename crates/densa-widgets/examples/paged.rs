//! A table of a billion rows in the grid widget, none of them held: a `PagedModel` fetches
//! the rows on screen a page at a time, on a thread of its own, as you scroll, drag the bar
//! or click a title to sort. The rows are computed when asked, each page after a short wait
//! as a query engine or a database would take, so you see them arrive.
//!
//! `cargo run --release -p densa-widgets --example paged` (`--latency MS` for each page's
//! wait, 40 unless told; `--light` for the light theme; `--seconds N` to close after N
//! seconds).

use std::error::Error;
use std::ops::Range;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use arrow_array::{
    ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray, TimestampMillisecondArray,
};
use densa_arrow::arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use densa_arrow::{FetchError, PageError, PageQuery, PageSource, PagedModel};
use densa_grid::{Column, GridModel, SortOrder};
use densa_ui::{Align, Style, Theme, Ui, WidgetId};
use densa_widgets::{DataGrid, Label, Panel};
use densa_window::{App, Host, WindowOptions};

/// Rows in the table.
const ROWS: u64 = 1_000_000_000;
/// Sensors: each holds `ROWS / SENSORS` rows (ids `s`, `s + 500`, ...).
const SENSORS: u64 = 500;
/// `value` is `id * SPREAD + SHIFT` modulo `ROWS`: a permutation, since the spread shares no
/// factor with `ROWS`, so the rows sorted by value are known without sorting.
const SPREAD: u64 = 7_919;
const SHIFT: u64 = 123_457;
/// When the first reading was taken; a reading every tenth of a second after.
const FIRST_MS: i64 = 1_700_000_000_000;
const EVERY_MS: i64 = 100;

type Grid = DataGrid<PagedModel>;

/// The billion rows, computed: `id`, `sensor`, `time`, `value` and `celsius` (which grows
/// with `value`), each sortable either way at the cost of a page.
struct Readings {
    latency: Duration,
    /// The inverse of `SPREAD` modulo `ROWS`.
    unspread: u64,
}

impl Readings {
    fn new(latency: Duration) -> Self {
        Self {
            latency,
            unspread: inverse(SPREAD, ROWS),
        }
    }

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("sensor", DataType::Utf8, false),
            Field::new(
                "time",
                DataType::Timestamp(TimeUnit::Millisecond, Some("+00:00".into())),
                false,
            ),
            Field::new("value", DataType::Int64, false),
            Field::new("celsius", DataType::Float64, false),
        ]))
    }

    /// The id of the row at `position` in `query`'s order.
    fn id_at(&self, query: &PageQuery, position: u64) -> u64 {
        let Some(key) = query.sort().first() else {
            return position;
        };
        let k = if key.is_descending() {
            ROWS - 1 - position
        } else {
            position
        };
        match key.column() {
            // The k-th smallest value is k: the id it came from.
            "value" | "celsius" => mul_mod(k + ROWS - SHIFT, self.unspread, ROWS),
            // Sensors in name order, each one's rows by id.
            "sensor" => {
                let per = ROWS / SENSORS;
                k / per + SENSORS * (k % per)
            }
            // `id` and `time` grow together.
            _ => k,
        }
    }
}

impl PageSource for Readings {
    fn row_count(&mut self, _: &PageQuery) -> Result<u64, PageError> {
        thread::sleep(self.latency);
        Ok(ROWS)
    }

    fn page(
        &mut self,
        query: &PageQuery,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, PageError> {
        thread::sleep(self.latency);
        // Lossless: a page is a few hundred rows.
        let end = offset.saturating_add(limit as u64).min(ROWS);
        let ids: Vec<u64> = (offset..end).map(|at| self.id_at(query, at)).collect();
        // Lossless: ids and values are below a billion.
        let id_column: Vec<i64> = ids.iter().map(|&id| id as i64).collect();
        let values: Vec<i64> = ids.iter().map(|&id| value_of(id) as i64).collect();
        let sensors: Vec<String> = ids
            .iter()
            .map(|id| format!("s-{:03}", id % SENSORS))
            .collect();
        let times: Vec<i64> = id_column
            .iter()
            .map(|&id| FIRST_MS + id * EVERY_MS)
            .collect();
        let celsius: Vec<f64> = values
            .iter()
            .map(|&v| -40.0 + v as f64 / ROWS as f64 * 100.0)
            .collect();
        let columns: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from(id_column)),
            Arc::new(StringArray::from(sensors)),
            Arc::new(TimestampMillisecondArray::from(times).with_timezone("+00:00")),
            Arc::new(Int64Array::from(values)),
            Arc::new(Float64Array::from(celsius)),
        ];
        Ok(vec![RecordBatch::try_new(Self::schema(), columns)?])
    }
}

/// The value of row `id`.
fn value_of(id: u64) -> u64 {
    (mul_mod(id, SPREAD, ROWS) + SHIFT) % ROWS
}

/// `a * b` modulo `m`, without overflow.
fn mul_mod(a: u64, b: u64, m: u64) -> u64 {
    // Lossless: the remainder is below `m`.
    (u128::from(a) * u128::from(b) % u128::from(m)) as u64
}

/// The inverse of `a` modulo `m` (they share no factor).
fn inverse(a: u64, m: u64) -> u64 {
    let (mut old, mut r) = (i128::from(a), i128::from(m));
    let (mut s, mut next) = (1_i128, 0_i128);
    while r != 0 {
        let q = old / r;
        (old, r) = (r, old - q * r);
        (s, next) = (next, s - q * next);
    }
    // Lossless: reduced modulo `m`, a `u64`.
    s.rem_euclid(i128::from(m)) as u64
}

struct PagedApp {
    light: bool,
    latency: Duration,
    close_at: Option<Instant>,
    ids: Option<Ids>,
    /// What the status bar tells now.
    status: Status,
}

struct Ids {
    grid: WidgetId,
    status: WidgetId,
}

/// What the status bar tells: how many rows there are, how many pages are held, which rows are
/// shown, in what order, and why the source failed, if it did. Written again only when it
/// changes, so a frame allocates nothing otherwise.
#[derive(Debug, Default)]
struct Status {
    rows: Option<u64>,
    pages: usize,
    shown: Range<usize>,
    sort: Option<(usize, SortOrder)>,
    error: Option<FetchError>,
}

impl Status {
    fn of(grid: &Grid) -> Self {
        let model = grid.model();
        Self {
            rows: model.row_count(),
            pages: model.pages_held(),
            shown: shown(grid),
            sort: grid.sort_order(),
            error: model.error().cloned(),
        }
    }

    /// Whether `grid` would tell what this tells (comparing, not copying).
    fn tells(&self, grid: &Grid) -> bool {
        let model = grid.model();
        self.rows == model.row_count()
            && self.pages == model.pages_held()
            && self.shown == shown(grid)
            && self.sort == grid.sort_order()
            && self.error.as_ref() == model.error()
    }

    /// The text, with the model's `columns` and `page_rows`.
    fn text(&self, columns: &[Column], page_rows: usize) -> String {
        if let Some(error) = &self.error {
            return error.to_string();
        }
        let Some(rows) = self.rows else {
            return "Counting the rows...".to_owned();
        };
        let shown = match self.shown.clone() {
            shown if shown.is_empty() => "no rows shown".to_owned(),
            shown => format!(
                "rows {} to {} shown",
                grouped(shown.start as u64),
                grouped(shown.end as u64 - 1)
            ),
        };
        let sorted = self
            .sort
            .and_then(|(column, order)| {
                let name = &columns.get(column)?.title;
                let way = match order {
                    SortOrder::Descending => "largest first",
                    _ => "smallest first",
                };
                Some(format!(" · sorted by {name}, {way}"))
            })
            .unwrap_or_default();
        format!(
            "{} rows · {} pages of {} held · {shown}{sorted}",
            grouped(rows),
            self.pages,
            page_rows
        )
    }
}

/// The rows on screen.
fn shown(grid: &Grid) -> Range<usize> {
    let first = grid.first_row();
    first..(first + grid.visible_rows()).min(grid.model().rows())
}

/// `n` with a comma between groups of three digits.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut text = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

impl App for PagedApp {
    fn build(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> Result<(), Box<dyn Error>> {
        if self.light {
            ui.set_theme(Theme::light());
        }
        let root = ui.root();
        let bar = Style::row()
            .padding(8.0)
            .gap(12.0)
            .shrink(0.0)
            .align_items(Align::Center);
        let toolbar = ui.add(root, Panel::new(), bar)?;
        ui.add(
            toolbar,
            Label::new("A billion rows").with_size(15.0),
            Style::row(),
        )?;
        let hint = "Scroll, drag the bar or press End: only the rows on screen are fetched. \
                    Click a title to sort.";
        ui.add(toolbar, Label::new(hint).dim(), Style::row())?;

        // The rows arrive on the model's thread: its wakes bring the frames that show them.
        let waker = host.waker();
        let source = Readings::new(self.latency);
        let mut model = PagedModel::new(Readings::schema(), source, move || {
            waker.wake();
        })?;
        model.set_decimals(4, Some(2))?;
        let grid = Grid::new(model).with_frozen_columns(1);
        let grid = ui.add(root, grid, Grid::style())?;

        let bar = Style::row().padding(6.0).shrink(0.0);
        let bar = ui.add(root, Panel::new(), bar)?;
        let text = self.status.text(&[], 0);
        let status = ui.add(bar, Label::new(text).dim(), Style::row())?;
        self.ids = Some(Ids { grid, status });
        Ok(())
    }

    fn frame(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> bool {
        if self.close_at.is_some_and(|at| Instant::now() >= at) {
            host.close();
            return false;
        }
        let Some(ids) = &self.ids else {
            return false;
        };
        // What arrived since the last frame, so the status bar tells it in this one.
        ui.update::<Grid, _>(ids.grid, |grid, cx| grid.model_mut(cx).prepare());
        let text = match ui.widget::<Grid>(ids.grid) {
            Some(grid) if !self.status.tells(grid) => {
                self.status = Status::of(grid);
                let model = grid.model();
                Some(self.status.text(model.columns(), model.page_rows()))
            }
            _ => None,
        };
        if let Some(text) = text {
            ui.update::<Label, _>(ids.status, |label, cx| label.set_text(cx, text));
        }
        // Frames come when pages arrive (their wakes) or the user acts; continuously only
        // to close by the clock.
        self.close_at.is_some()
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .skip_while(|a| *a != flag)
            .nth(1)
            .map(|s| s.parse::<u64>())
            .transpose()
    };
    let light = args.iter().any(|a| a == "--light");
    let seconds = value("--seconds")?;
    let latency = value("--latency")?.unwrap_or(40);
    let app = PagedApp {
        light,
        latency: Duration::from_millis(latency),
        close_at: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        ids: None,
        status: Status::default(),
    };
    densa_window::run(
        WindowOptions::new("densa: a billion rows").with_size(1000.0, 700.0),
        app,
    )?;
    Ok(())
}
