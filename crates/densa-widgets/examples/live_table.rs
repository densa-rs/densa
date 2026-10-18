//! A live table in the grid widget: 100,000 simulated instruments in a `densa-data` table,
//! a thousand of them updated every frame. Click a title to sort (the view stays sorted as
//! prices move), type in the box to filter by symbol, click a row to select it: the
//! selection follows its instrument wherever it moves.
//!
//! `cargo run --release -p densa-widgets --example live_table` (`--light` for the light
//! theme, `--seconds N` to close after N seconds).

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use densa_arrow::LiveModel;
use densa_data::{Filter, LiveTable};
use densa_grid::GridModel;
use densa_ui::{Action, ActionKind, Align, Style, Theme, Ui, WidgetId};
use densa_widgets::{DataGrid, Label, Panel, TextInput};
use densa_window::{App, Host, WindowOptions};

/// Instruments in the table.
const INSTRUMENTS: usize = 100_000;
/// Instruments updated each frame.
const UPDATES_PER_FRAME: usize = 1_000;
/// How often the status bar counts the rows again.
const STATUS_EVERY: Duration = Duration::from_millis(500);
const SECTORS: [&str; 8] = [
    "Energy",
    "Materials",
    "Industrials",
    "Utilities",
    "Health",
    "Financials",
    "Technology",
    "Telecom",
];

type Grid = DataGrid<LiveModel>;

/// Random prices and volumes: wyrand, deterministic.
struct Feed {
    seed: u64,
}

impl Feed {
    fn next(&mut self) -> u64 {
        self.seed = self.seed.wrapping_add(0xA076_1D64_78BD_642F);
        let t = u128::from(self.seed) * u128::from(self.seed ^ 0xE703_7ED1_A0B4_28DB);
        // Lossless: the halves of a 128-bit product.
        (t >> 64) as u64 ^ t as u64
    }

    /// A number in `0..1`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }

    /// The instruments `indices`, with prices and volumes drawn at random around where each
    /// one opened.
    fn batch(
        &mut self,
        indices: impl Iterator<Item = usize>,
    ) -> Result<RecordBatch, Box<dyn Error>> {
        let mut symbols = Vec::new();
        let mut sectors = Vec::new();
        let (mut last, mut change, mut volume) = (Vec::new(), Vec::new(), Vec::new());
        for index in indices {
            let open = open_price(index);
            let move_pct = (self.unit() - 0.5) * 8.0;
            symbols.push(format!("SYM{index:05}"));
            sectors.push(SECTORS[index % SECTORS.len()]);
            last.push(open * (1.0 + move_pct / 100.0));
            change.push(move_pct);
            // Lossless: below 10 million.
            volume.push((self.unit() * 10_000_000.0) as i64);
        }
        Ok(RecordBatch::try_from_iter([
            ("symbol", Arc::new(StringArray::from(symbols)) as ArrayRef),
            ("sector", Arc::new(StringArray::from(sectors)) as ArrayRef),
            ("last", Arc::new(Float64Array::from(last)) as ArrayRef),
            ("chg %", Arc::new(Float64Array::from(change)) as ArrayRef),
            ("volume", Arc::new(Int64Array::from(volume)) as ArrayRef),
        ])?)
    }
}

/// Where instrument `index` opened: spread from 5 to about 1,000.
fn open_price(index: usize) -> f64 {
    5.0 + (index * 7_919 % 99_500) as f64 / 100.0
}

struct LiveApp {
    light: bool,
    close_at: Option<Instant>,
    feed: Feed,
    ids: Option<Ids>,
    status_at: Instant,
}

struct Ids {
    grid: WidgetId,
    filter: WidgetId,
    status: WidgetId,
}

impl LiveApp {
    /// The model: every instrument once, the prices with two decimals.
    fn model(&mut self) -> Result<LiveModel, Box<dyn Error>> {
        let first = self.feed.batch(0..INSTRUMENTS)?;
        let mut table = LiveTable::new(first.schema(), "symbol")?;
        table.upsert(first)?;
        let mut model = LiveModel::new(table)?;
        model.set_decimals(2, Some(2))?;
        model.set_decimals(3, Some(2))?;
        Ok(model)
    }

    /// Counts the rows and names the selected instrument.
    fn show_status(ui: &mut Ui, ids: &Ids) {
        let Some(grid) = ui.widget::<Grid>(ids.grid) else {
            return;
        };
        let model = grid.model();
        let shown = model.rows();
        let total = model.table().len();
        let selected = grid
            .selected()
            .and_then(|row| model.record(row))
            .and_then(|record| model.table().row_of(record))
            .and_then(|at| {
                let symbols = model.table().batches()[at.batch].column(0);
                let symbols = symbols.as_any().downcast_ref::<StringArray>()?;
                Some(format!(", selected {}", symbols.value(at.row)))
            })
            .unwrap_or_default();
        let text = format!("{total} instruments, {shown} shown{selected}");
        ui.update::<Label, _>(ids.status, |label, cx| label.set_text(cx, text));
    }

    /// Shows only the instruments whose symbol contains the filter text.
    fn apply_filter(ui: &mut Ui, ids: &Ids) {
        let text = ui
            .widget::<TextInput>(ids.filter)
            .map(|input| input.text().trim().to_uppercase())
            .unwrap_or_default();
        let filter = (!text.is_empty()).then(|| Filter::contains("symbol", text));
        ui.update::<Grid, _>(ids.grid, |grid, cx| {
            // A contains filter on a string column always compiles.
            let _ = grid.model_mut(cx).set_filter(filter);
        });
    }
}

impl App for LiveApp {
    fn build(&mut self, ui: &mut Ui, _: &mut Host<'_>) -> Result<(), Box<dyn Error>> {
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
            Label::new("Live table").with_size(15.0),
            Style::row(),
        )?;
        let filter = TextInput::new("").with_placeholder("Filter symbols");
        let filter = ui.add(toolbar, filter, Style::row())?;

        let grid = Grid::new(self.model()?).with_frozen_columns(1);
        let grid = ui.add(root, grid, Grid::style())?;

        let bar = Style::row().padding(6.0).shrink(0.0);
        let bar = ui.add(root, Panel::new(), bar)?;
        let status = ui.add(bar, Label::new("").dim(), Style::row())?;
        let ids = Ids {
            grid,
            filter,
            status,
        };
        Self::show_status(ui, &ids);
        self.ids = Some(ids);
        Ok(())
    }

    fn action(&mut self, ui: &mut Ui, _: &mut Host<'_>, action: Action) {
        let Some(ids) = &self.ids else {
            return;
        };
        if action.source == ids.filter && matches!(action.kind, ActionKind::Changed) {
            Self::apply_filter(ui, ids);
        }
        Self::show_status(ui, ids);
    }

    fn frame(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> bool {
        let Some(ids) = &self.ids else {
            return false;
        };
        let start = (self.feed.next() % INSTRUMENTS as u64) as usize;
        let indices = (0..UPDATES_PER_FRAME).map(|i| (start + i * 97) % INSTRUMENTS);
        if let Ok(batch) = self.feed.batch(indices) {
            ui.update::<Grid, _>(ids.grid, |grid, cx| {
                // The batch has the table's schema: an upsert cannot fail.
                let _ = grid.model_mut(cx).table_mut().upsert(batch);
            });
        }
        if self.status_at.elapsed() >= STATUS_EVERY {
            self.status_at = Instant::now();
            Self::show_status(ui, ids);
        }
        if self.close_at.is_some_and(|at| Instant::now() >= at) {
            host.close();
            return false;
        }
        true
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let light = args.iter().any(|a| a == "--light");
    let seconds = args
        .iter()
        .skip_while(|a| *a != "--seconds")
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let app = LiveApp {
        light,
        close_at: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        feed: Feed { seed: 0x5EED },
        ids: None,
        status_at: Instant::now(),
    };
    densa_window::run(
        WindowOptions::new("densa live table").with_size(1000.0, 700.0),
        app,
    )?;
    Ok(())
}
