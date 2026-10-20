//! A docking workspace: panels in tab groups that you rearrange by dragging their tabs onto
//! other groups or near their edges, and resize by dragging the gaps. The watchlist is a live
//! data grid of simulated instruments: click a title to sort, a row to select it.
//!
//! `cargo run --release -p densa-widgets --example dock` (`--light` for the light theme,
//! `--seconds N` to close after N seconds, `--demo` to start with a tab being dragged,
//! `--layout FILE` to start from the arrangement saved in FILE and save every change to it).

use std::error::Error;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use densa_core::Point;
use densa_grid::{Align as Side, CellStyle, CellText, Column, GridModel, Palette, SortOrder};
use densa_ui::{
    Action, ActionKind, Align, PointerButton, PointerEvent, PointerKind, Style, Theme, Ui, WidgetId,
};
use densa_widgets::{
    DataGrid, DockArea, DockAt, DockLayout, DockPanel, DockSide, Label, Panel, ScrollView,
};
use densa_window::{App, Host, WindowOptions};

/// Instruments in the watchlist.
const NAMES: [&str; 12] = [
    "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "AVGO", "ASML", "AMD", "INTC", "ORCL",
];

/// Instruments in the live watchlist.
const INSTRUMENTS: usize = 2_000;
/// Price changes per frame.
const TICKS_PER_FRAME: usize = 20;
/// How long a changed price stays lit, in seconds.
const FLASH: f32 = 0.3;

struct Workspace {
    light: bool,
    demo: bool,
    close_at: Option<Instant>,
    started: Instant,
    dock: Option<WidgetId>,
    status: Option<WidgetId>,
    watchlist: Option<WidgetId>,
    /// Where the arrangement is saved, if anywhere.
    layout_file: Option<PathBuf>,
}

/// One simulated instrument.
struct Quote {
    symbol: String,
    open: f64,
    last: f64,
    volume: u64,
    /// When the price last changed, in seconds, and whether it rose.
    changed: (f32, bool),
}

/// Simulated instruments whose prices walk at random: a grid model.
struct Market {
    columns: Vec<Column>,
    quotes: Vec<Quote>,
    seed: u64,
    now: f32,
}

impl Market {
    fn new() -> Self {
        let quotes = (0..INSTRUMENTS)
            .map(|i| {
                let name = NAMES[i % NAMES.len()];
                let open = 20.0 + (i * 37 % 900) as f64 + (i * 13 % 100) as f64 / 100.0;
                Quote {
                    symbol: format!("{name}.{:04}", i / NAMES.len()),
                    open,
                    last: open,
                    volume: (i as u64 * 7_919) % 90_000 + 1_000,
                    changed: (f32::NEG_INFINITY, true),
                }
            })
            .collect();
        Self {
            columns: vec![
                Column::new("Symbol", 10, Side::Left),
                Column::new("Last", 9, Side::Right),
                Column::new("Chg %", 7, Side::Right),
                Column::new("Volume", 9, Side::Right),
                Column::new("Open", 9, Side::Right),
            ],
            quotes,
            seed: 0x9E37_79B9_7F4A_7C15,
            now: 0.0,
        }
    }

    /// The next pseudo-random number (wyrand).
    fn next(&mut self) -> u64 {
        self.seed = self.seed.wrapping_add(0xa076_1d64_78bd_642f);
        let t = u128::from(self.seed) * u128::from(self.seed ^ 0xe703_7ed1_a0b4_28db);
        (t as u64) ^ ((t >> 64) as u64)
    }

    /// Moves `count` prices a tick up or down at time `now`.
    fn tick(&mut self, now: f32, count: usize) {
        self.now = now;
        for _ in 0..count {
            let r = self.next();
            let i = (r % self.quotes.len() as u64) as usize;
            let up = r & (1 << 40) != 0;
            let quote = &mut self.quotes[i];
            let step = (quote.last * 0.0005).max(0.01);
            quote.last = (quote.last + if up { step } else { -step }).max(0.01);
            quote.volume += 1 + (r >> 50) % 500;
            quote.changed = (now, up);
        }
    }
}

impl GridModel for Market {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn rows(&self) -> usize {
        self.quotes.len()
    }

    fn cell(&self, row: usize, column: usize, text: &mut CellText, palette: &Palette) -> CellStyle {
        let quote = &self.quotes[row];
        let change = (quote.last / quote.open - 1.0) * 100.0;
        // Below the shown precision: unchanged, not "-0.00".
        let change = if change.abs() < 0.005 { 0.0 } else { change };
        let direction = match change {
            c if c > 0.0 => palette.positive,
            c if c < 0.0 => palette.negative,
            _ => palette.text,
        };
        let _ = match column {
            0 => text.write_str(&quote.symbol),
            1 => write!(text, "{:.2}", quote.last),
            2 if change == 0.0 => text.write_str("0.00"),
            2 => write!(text, "{change:+.2}"),
            3 => write!(text, "{}", quote.volume),
            _ => write!(text, "{:.2}", quote.open),
        };
        let (when, up) = quote.changed;
        let lit = column == 1 && self.now - when < FLASH;
        let flash = if up {
            palette.positive
        } else {
            palette.negative
        }
        .with_alpha(0.35);
        CellStyle::new(if column == 2 { direction } else { palette.text })
            .with_background(lit.then_some(flash))
    }

    fn sort(&mut self, column: usize, order: SortOrder) -> bool {
        let key = |q: &Quote| match column {
            1 => q.last,
            2 => q.last / q.open,
            3 => q.volume as f64,
            4 => q.open,
            _ => 0.0,
        };
        if column == 0 {
            self.quotes.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        } else {
            self.quotes.sort_by(|a, b| key(a).total_cmp(&key(b)));
        }
        if order == SortOrder::Descending {
            self.quotes.reverse();
        }
        true
    }
}

/// A panel holding a scrolling list of `rows`, keyed by its title in lower case.
fn list_panel(
    ui: &mut Ui,
    dock: WidgetId,
    title: &str,
    at: DockAt,
    rows: impl Iterator<Item = String>,
) -> Result<WidgetId, Box<dyn Error>> {
    let spec = DockPanel::new(title).with_key(title.to_lowercase());
    let panel = DockArea::add_panel(ui, dock, Panel::new(), spec, at)?;
    let list = ui.add(panel, ScrollView::new(), ScrollView::style().padding(6.0))?;
    for row in rows {
        ui.add(list, Label::new(row), Style::row().padding(3.0).shrink(0.0))?;
    }
    Ok(panel)
}

/// A panel holding a heading and a wrapped paragraph.
fn text_panel(
    ui: &mut Ui,
    dock: WidgetId,
    title: &str,
    at: DockAt,
    text: &str,
) -> Result<WidgetId, Box<dyn Error>> {
    let spec = DockPanel::new(title).with_key(title.to_lowercase());
    let panel = DockArea::add_panel(ui, dock, Panel::new(), spec, at)?;
    let body = Style::column().grow(1.0).padding(14.0).gap(8.0);
    let body = ui.add(panel, Panel::transparent(), body)?;
    ui.add(body, Label::new(title).with_size(18.0), Style::row())?;
    ui.add(body, Label::new(text).wrapping().dim(), Style::row())?;
    Ok(panel)
}

fn quote(index: usize) -> String {
    let name = NAMES[index % NAMES.len()];
    let price = 40.0 + (index * 37 % 500) as f64 + (index * 13 % 100) as f64 / 100.0;
    let change = (index * 29 % 400) as f64 / 100.0 - 2.0;
    format!("{name:<7}{price:>9.2}{change:>+7.2}%")
}

fn log_line(index: usize) -> String {
    let (h, m, s) = (9 + index / 60 % 8, index % 60, index * 7 % 60);
    format!("{h:02}:{m:02}:{s:02}  event {index:>4}  ok")
}

impl App for Workspace {
    fn build(&mut self, ui: &mut Ui, _: &mut Host<'_>) -> Result<(), Box<dyn Error>> {
        if self.light {
            ui.set_theme(Theme::light());
        }
        let root = ui.root();
        let dock = ui.add(root, DockArea::new(), DockArea::style().padding(0.0))?;
        let spec = DockPanel::new("Watchlist").with_key("watchlist");
        let watch = DockArea::add_panel(ui, dock, Panel::new(), spec, DockAt::First)?;
        let grid = DataGrid::new(Market::new()).with_frozen_columns(1);
        self.watchlist = Some(ui.add(watch, grid, DataGrid::<Market>::style())?);
        let about = "Drag a tab along its bar to reorder, onto another group to join it, or \
                     near a group's edge to split the group. Drag the gaps between groups to \
                     resize them. Painting and dragging tabs allocate nothing.";
        let details = text_panel(
            ui,
            dock,
            "Details",
            DockAt::Beside(watch, DockSide::Right),
            about,
        )?;
        let below = DockAt::Beside(details, DockSide::Bottom);
        let log = list_panel(ui, dock, "Log", below, (0..200).map(log_line))?;
        text_panel(ui, dock, "Notes", DockAt::With(log), "Tabs share a group.")?;
        list_panel(
            ui,
            dock,
            "Screener",
            DockAt::With(watch),
            (12..40).map(quote),
        )?;
        ui.update::<DockArea, _>(dock, |area, cx| area.show(cx, watch));

        let bar = Style::row()
            .padding(6.0)
            .shrink(0.0)
            .align_items(Align::Center);
        let bar = ui.add(root, Panel::new(), bar)?;
        let status = ui.add(bar, Label::new("Ready").dim(), Style::row())?;
        self.status = Some(status);
        self.dock = Some(dock);
        if let Some(text) = self.read_layout(ui) {
            ui.update::<Label, _>(status, |label, cx| label.set_text(cx, text));
        }
        Ok(())
    }

    fn action(&mut self, ui: &mut Ui, _: &mut Host<'_>, action: Action) {
        let (Some(dock), Some(status)) = (self.dock, self.status) else {
            return;
        };
        if Some(action.source) == self.watchlist {
            if let ActionKind::Selected(row) = action.kind {
                let symbol = ui
                    .widget::<DataGrid<Market>>(action.source)
                    .and_then(|g| g.model().quotes.get(row))
                    .map(|q| q.symbol.clone())
                    .unwrap_or_default();
                ui.update::<Label, _>(status, |label, cx| {
                    label.set_text(cx, format!("Selected {symbol}"))
                });
            }
            return;
        }
        if action.source != dock {
            return;
        }
        let groups = ui.widget::<DockArea>(dock).map_or(0, DockArea::group_count);
        let Some(area) = ui.widget::<DockArea>(dock) else {
            return;
        };
        let saved =
            self.layout_file.as_ref().zip(area.layout()).map(
                |(file, layout)| match std::fs::write(file, layout.to_string()) {
                    Ok(()) => String::from(", saved"),
                    Err(error) => format!(", not saved: {error}"),
                },
            );
        let text = match action.kind {
            ActionKind::Selected(index) => {
                let title = area.panels().nth(index).and_then(|p| area.title(p));
                format!("Showing {}", title.unwrap_or("?"))
            }
            ActionKind::Changed => format!("Rearranged: {groups} groups"),
            _ => return,
        };
        let text = text + saved.as_deref().unwrap_or("");
        ui.update::<Label, _>(status, |label, cx| label.set_text(cx, text));
    }

    fn frame(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> bool {
        if let Some(grid) = self.watchlist {
            let now = self.started.elapsed().as_secs_f32();
            ui.update::<DataGrid<Market>, _>(grid, |grid, cx| {
                grid.model_mut(cx).tick(now, TICKS_PER_FRAME);
            });
        }
        if std::mem::take(&mut self.demo) {
            // The Screener tab held over the right of the Log group, once the window has its
            // size (it has none yet in `build`).
            if let Some((from, to)) = self.dock.and_then(|dock| demo_drag(ui, dock)) {
                let press = PointerKind::Down(PointerButton::Primary);
                ui.pointer(PointerEvent::new(press, from));
                ui.pointer(PointerEvent::new(PointerKind::Move, to));
            }
        }
        if self.close_at.is_some_and(|at| Instant::now() >= at) {
            host.close();
            return false;
        }
        // The prices keep moving: paint every frame.
        true
    }
}

impl Workspace {
    /// Arranges the dock as the layout file says, if there is one: what to tell the user.
    fn read_layout(&self, ui: &mut Ui) -> Option<String> {
        let file = self.layout_file.as_ref()?;
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => return Some(format!("Layout not read: {error}")),
        };
        match text.parse::<DockLayout>() {
            Ok(layout) => {
                let restored = DockArea::restore_layout(ui, self.dock?, &layout);
                Some(match restored {
                    Ok(()) => format!("Layout restored from {}", file.display()),
                    Err(error) => format!("Layout not restored: {error}"),
                })
            }
            Err(error) => Some(format!("Layout not restored: {error}")),
        }
    }
}

/// Where `--demo` presses (the Screener tab) and where it holds the tab (near the right edge
/// of the group the Log panel is in).
fn demo_drag(ui: &mut Ui, dock: WidgetId) -> Option<(Point, Point)> {
    ui.layout();
    let area = ui.widget::<DockArea>(dock)?;
    let screener = area.panel_with_key("screener")?;
    let tab = area.tab_rect(screener, ui.rect(dock)?, ui.scale())?;
    let log = area.panel_with_key("log")?;
    let shown = area.tabs_with(log).find(|&p| area.is_shown(p))?;
    let group = ui.rect(shown)?;
    let from = Point::new(
        (tab.left() + tab.right()) / 2.0,
        (tab.top() + tab.bottom()) / 2.0,
    );
    let to = Point::new(
        group.right() - group.width() / 10.0,
        (group.top() + group.bottom()) / 2.0,
    );
    Some((from, to))
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let light = args.iter().any(|a| a == "--light");
    let demo = args.iter().any(|a| a == "--demo");
    let seconds = args
        .iter()
        .skip_while(|a| *a != "--seconds")
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let app = Workspace {
        light,
        demo,
        close_at: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        started: Instant::now(),
        dock: None,
        status: None,
        watchlist: None,
        layout_file: args
            .iter()
            .skip_while(|a| *a != "--layout")
            .nth(1)
            .map(PathBuf::from),
    };
    densa_window::run(
        WindowOptions::new("densa dock").with_size(1200.0, 760.0),
        app,
    )?;
    Ok(())
}
