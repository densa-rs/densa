<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/densa-rs/densa/main/docs/brand/densa-logo-dark.svg">
    <img alt="densa" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/brand/densa-logo.svg" width="320">
  </picture>
</h1>

<p align="center"><b>Dense by design.</b></p>

<p align="center">
  <a href="https://crates.io/crates/densa"><img alt="crates.io" src="https://img.shields.io/crates/v/densa?color=2F5BEA"></a>
  <a href="https://docs.rs/densa"><img alt="docs.rs" src="https://img.shields.io/docsrs/densa"></a>
  <a href="https://github.com/densa-rs/densa/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/densa-rs/densa/actions/workflows/ci.yml/badge.svg"></a>
  <a href="#license"><img alt="License: MIT or Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-2F5BEA"></a>
  <img alt="Rust 1.90+" src="https://img.shields.io/badge/rust-1.90%2B-2F5BEA">
  <a href="https://densa.dev"><img alt="Website: densa.dev" src="https://img.shields.io/badge/web-densa.dev-0B1220"></a>
</p>

densa is a Rust toolkit for applications that show a lot of live data: trading terminals,
monitoring consoles, operational dashboards, developer tools. It keeps live data in order as
it changes, and draws it on the GPU at the display's refresh rate, from the data stream to
the pixel.

> **Status: 0.x, experimental.** On [crates.io](https://crates.io/crates/densa); APIs will change
> between releases. Builds and tests on Linux, macOS and Windows.

## Architecture

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/densa-rs/densa/main/docs/architecture-dark.svg">
    <img alt="densa's crates in three lanes: data (Arrow sources, densa-data, densa-arrow), interface (densa-grid, densa-chart, densa-widgets, densa-paragraph, densa-ui, densa-text, densa-draw, densa-render, densa-window, the GPU) and runtime (densa-sync, densa-platform, densa-fmt, densa-alloc), all on densa-core" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/architecture.svg" width="900">
  </picture>
</p>

- **Data.** Arrow record batches from files, streams or query engines go into live tables;
  views sort and filter them, groups aggregate them, and both follow every change
  incrementally.
- **Interface.** A tree of widgets, laid out with flexbox, takes pointer, keyboard and input
  method events; widgets (buttons, text inputs, scroll views, a data grid, charts) draw into a
  display list that is independent of any GPU API, and `densa-window` presents a whole frame
  in one draw call with wgpu.
- **Runtime.** Hand data between threads without locks, place threads on the right cores,
  format numbers without allocating, and prove that frames never allocate.

Each crate is usable on its own, and only `densa-render` and `densa-window` depend on a GPU
or windowing crate.

## Highlights

- **Live tables.** Upsert Arrow batches by key; sorted, filtered views and grouped
  aggregates are kept up to date as rows change instead of being recomputed.
- **Charts over live data.** Lines, candlesticks and volume in stacked panes that follow the
  newest data, zoom around the pointer and pan; a line is drawn one pixel column at a time,
  so a million points cost about as much as the chart is wide and every high and low shows.
- **Widgets with flexbox layout.** A retained widget tree laid out with flexbox, with pointer
  capture, hover, focus and Tab order, and widgets for toolbars, forms, lists, split panes
  and docking workspaces; it runs headless, so interfaces are tested without a window.
- **Accessible.** Widgets describe themselves to screen readers (through AccessKit on
  Windows, macOS and Linux) and do what they are asked: press, type, scroll, choose a tab.
  Docking tabs are reachable and usable from the keyboard.
- **Arrow from end to end.** Batches are shared, never copied, from the source to the
  screen; a grid over millions of rows costs only its visible rows.
- **No allocation per frame.** Drawing, scrolling, hit-testing, delivering events and moving
  a caret allocate nothing once fonts and glyphs are loaded; tests prove it with a counting
  allocator.
- **No locks between threads.** A wait-free triple buffer hands the newest data to the UI
  thread; its memory orderings are model-checked with loom.
- **Real text.** Shaping, wrapping, bidirectional text, selection, editing and input methods
  (IME), built on [parley](https://github.com/linebender/parley). Characters the bundled
  font lacks, such as Korean, Japanese and Chinese, come from the system's fonts.
- **One draw call per frame**, with a glyph atlas whose memory is bounded.

## Quick start

You need Rust 1.90 or later and a GPU with Vulkan, Metal, DirectX 12 or OpenGL.

```sh
git clone https://github.com/densa-rs/densa
cd densa
cargo run --release -p densa-widgets --example monitor      # a live fleet monitor: grid and chart
cargo run --release -p densa-widgets --example gallery      # widgets in a window
cargo run --release -p densa-widgets --example dock         # a docking workspace, live grid
cargo run --release -p densa-widgets --example live_table   # a live table in the grid widget
cargo run --release -p densa-widgets --example chart        # live candles, volume, pan and zoom
cargo run --release -p densa-widgets --example linked       # a grid and a chart over live tables
cargo run --release -p densa-arrow --example live            # a live table, sorted as it moves
cargo run --release -p densa-arrow --example table           # a million-row Arrow table
cargo run --release -p densa-paragraph --example editor      # editable text
cargo run --release -p trading-demo                          # a streaming market grid
```

To use densa, depend on the `densa` crate, which gathers the libraries as modules
(`densa::data`, `densa::widgets`, `densa::window`, ...):

```toml
[dependencies]
densa = "0.1"
```

Or take only the crates you use, each on its own (`densa-data` needs no GPU or window):

```toml
[dependencies]
densa-data = "0.1"
densa-widgets = "0.1"
densa-window = "0.1"
```

## Crates

| Crate | Use it for |
|---|---|
| [`densa`](crates/densa) | All of the crates below under one name, as modules; the `window` and `data` features (on by default) leave out the GPU and Arrow when off |
| [`densa-data`](crates/densa-data) | Live Arrow tables: keyed upserts and deletes, sorted and filtered views and grouped aggregates kept up to date incrementally |
| [`densa-arrow`](crates/densa-arrow) | Arrow tables and live views in the data grid, the grid widget and charts, with typed, allocation-free cell formatting |
| [`densa-ui`](crates/densa-ui) | The widget tree: flexbox layout, pointer, keyboard and input method events, focus, themes, an accessibility tree; no window or GPU needed |
| [`densa-widgets`](crates/densa-widgets) | Labels, buttons, text inputs, scroll views, splitters, panels, docking, menus and context menus, a data grid widget, a chart widget |
| [`densa-window`](crates/densa-window) | Running an interface in a native window: winit input in, a wgpu frame out, screen readers through AccessKit |
| [`densa-grid`](crates/densa-grid) | A data grid over your own data (`GridModel`, `GridSource`): columns, alignment, cell styles |
| [`densa-chart`](crates/densa-chart) | Charts over your own data (`Points`, `Candles`, `ChartModel`): lines, candlesticks, bars, number and time axes, a crosshair |
| [`densa-paragraph`](crates/densa-paragraph) | Paragraphs: shaping, wrapping, bidirectional text, selection, keyboard navigation, editing with input methods |
| [`densa-text`](crates/densa-text) | Fonts, glyph rasterization and a glyph atlas that fits the GPU's limits |
| [`densa-draw`](crates/densa-draw) | A frame's display list: a `Canvas` that batches rectangles and glyphs, independent of any GPU API |
| [`densa-render`](crates/densa-render) | Drawing a display list with wgpu in one draw call |
| [`densa-sync`](crates/densa-sync) | A wait-free triple buffer: the latest data from a producer thread to the UI thread |
| [`densa-platform`](crates/densa-platform) | CPU topology, thread placement and priority (Windows today) |
| [`densa-fmt`](crates/densa-fmt) | Number formatting into stack buffers, without allocating |
| [`densa-alloc`](crates/densa-alloc) | A counting global allocator, to prove a code path allocates nothing |
| [`densa-core`](crates/densa-core) | Shared primitives: `Color`, `Point`, `Rect`, `Size` |

Connectors that bring data in, with the large dependency trees their formats and protocols
need, live in [densa-connectors](https://github.com/densa-rs/densa-connectors): today
`densa-files`, which reads CSV and Parquet files as Arrow batches on a background thread, and
`densa-ws`, which reads WebSocket streams of JSON rows or Arrow IPC as Arrow batches and
reconnects on its own, and `densa-flight`, which reads Arrow Flight and Flight SQL results.

## Usage

### A live table, a sorted view and aggregates

```rust
use densa_data::{
    Aggregate, CompareOp, Filter, GroupSpec, LiveTable, Order, Scalar, ViewSpec,
};

let mut table = LiveTable::new(schema, "symbol")?; // keyed by the `symbol` column
let movers = table.add_view(
    ViewSpec::new()
        .filter(Filter::compare("volume", CompareOp::Gt, Scalar::Int(1_000)))
        .sort_by("change_pct", Order::Descending),
)?;

table.upsert(ticks)?; // a RecordBatch: new symbols insert, known ones update
table.delete(&delisted)?; // an array of keys

let view = table.view(movers).unwrap(); // in order, after every change
let top = view.row(0); // where the row's values are in table.batches()

// Grouped aggregates follow every change the same way.
let by_sector = table.add_groups(
    GroupSpec::new()
        .group_by("sector")
        .aggregate(Aggregate::count())
        .aggregate(Aggregate::sum("volume").named("volume")),
)?;
let summary = table.groups(by_sector).unwrap().to_batch()?; // one row per sector
```

To show it in the grid widget, `densa_arrow::LiveModel` wraps the table: a click on a column
title sorts the view, the app sets a filter, and the selection follows its row as values
change (every row has a `RecordId` that lasts as long as its key). The
[`live_table` example](crates/densa-widgets/examples/live_table.rs) does this with 100,000
instruments, a thousand of them updated every frame; `densa_arrow::LiveGrid` paints a view
without widgets.

### Widgets in a window

```rust
use densa_ui::{Action, ActionKind, Align, Style, Ui, WidgetId};
use densa_widgets::{Button, Label, Panel, TextInput};
use densa_window::{App, Host, WindowOptions};

#[derive(Default)]
struct Search {
    query: Option<WidgetId>,
    run: Option<WidgetId>,
}

impl App for Search {
    fn build(&mut self, ui: &mut Ui, _: &mut Host<'_>) -> Result<(), Box<dyn std::error::Error>> {
        let bar = Style::row().gap(6.0).padding(8.0).align_items(Align::Center);
        let bar = ui.add(ui.root(), Panel::new(), bar)?;
        ui.add(bar, Label::new("Symbol"), Style::row())?;
        let input = TextInput::new("").with_placeholder("AAPL");
        self.query = Some(ui.add(bar, input, Style::row())?);
        self.run = Some(ui.add(bar, Button::new("Run").primary(), Style::row())?);
        Ok(())
    }

    // Called with what widgets report: a click, an edit.
    fn action(&mut self, ui: &mut Ui, _: &mut Host<'_>, action: Action) {
        if action.kind == ActionKind::Clicked && Some(action.source) == self.run {
            let query = self.query.and_then(|id| ui.widget::<TextInput>(id));
            println!("search {}", query.map_or("", |input| input.text()));
        }
    }
}

densa_window::run(WindowOptions::new("Search"), Search::default())?;
```

A `Ui` also runs without a window: build it, feed it events and read its actions in tests.

### A grid over your own data

```rust
use densa_fmt::write_fixed;
use densa_grid::{CellStyle, CellText, Color, GridSource};
use std::fmt::Write as _;

/// A monitoring table: one row per sensor.
struct Sensors<'a> {
    names: &'a [&'a str],
    celsius: &'a [f64],
}

impl GridSource for Sensors<'_> {
    fn rows(&self) -> usize {
        self.celsius.len()
    }

    // Called for every visible cell, every frame.
    fn cell(&self, row: usize, column: usize, text: &mut CellText) -> CellStyle {
        let value = self.celsius[row];
        if column == 0 {
            let _ = text.write_str(self.names[row]);
        } else {
            let _ = write_fixed(text, value, 1);
        }
        let hex = if value > 80.0 { 0xF87171 } else { 0xE2E8F0 };
        CellStyle { color: Color::srgb(hex), background: None }
    }
}
```

`GridPainter::paint` draws a `GridSource` onto a `Canvas`; `GpuRenderer` draws the result.

### A chart over your own data

```rust
use densa_chart::{paint_line, points_within, y_extent, Axis, Color, LineStyle, Plot, Rect};

// A day of readings, one a second: (seconds since the epoch, value).
let readings: Vec<(f64, f64)> = (0..86_400)
    .map(|s| (1_792_368_000.0 + f64::from(s), (f64::from(s) / 3_600.0).sin()))
    .collect();

// The last hour, scaled to what it holds.
let x = Axis::time(1_792_450_800.0, 1_792_454_400.0);
let (low, high) = y_extent(&readings[..], points_within(&readings[..], &x)).unwrap();
let plot = Plot::new(Rect::new(0.0, 0.0, 800.0, 300.0), x, Axis::number(low, high).padded(0.05));

// One span per pixel column, however many readings fall in it.
paint_line(&mut canvas, &plot, &readings[..], LineStyle::new(Color::WHITE));
```

`ChartPainter` adds gridlines, tick labels and a crosshair; `paint_candles` and `paint_bars`
draw candlesticks and volume. In a window, implement `ChartModel` (stacked panes on one time
axis) and give it to the `Chart` widget, which follows new data, zooms with the wheel and
pans by dragging: see the [`chart` example](crates/densa-widgets/examples/chart.rs).
For live data, `densa_arrow::LiveSeries` reads two numeric columns of a live table's view
(numbers, timestamps, dates) as a series, so a chart keeps up with the same table a grid
shows: see the [`linked` example](crates/densa-widgets/examples/linked.rs).

### Text

```rust
use densa_paragraph::{Motion, ParagraphStyle, TextAlign, TextEditor, TextSystem};

let mut text = TextSystem::new()?; // fonts and layout scratch space, once per application

// A paragraph: shaped once, re-wrapped to any width without shaping again.
let style = ParagraphStyle::new(15.0).with_align(TextAlign::Justify).with_max_width(Some(480.0));
let mut paragraph = text.paragraph("Shaped once, wrapped to any width.", &style, &[])?;
paragraph.reflow(Some(320.0))?;

// Editable text: typing, deletion by character or word, input-method composition.
let mut editor = TextEditor::new(&mut text, "Hello", ParagraphStyle::new(15.0))?;
editor.insert(&mut text, ", brave world")?;
editor.delete(&mut text, Motion::PreviousWord)?; // ctrl+Backspace
```

`TextSelection` turns clicks into caret positions and moves by character, word and line, in
screen order for arrow keys and in text order for deletion, so right-to-left text behaves.
See the [`paragraphs`](crates/densa-paragraph/examples/paragraphs.rs) and
[`editor`](crates/densa-paragraph/examples/editor.rs) examples.

### Handing data to the UI thread

```rust
use densa_sync::triple_buffer;

let (mut writer, mut reader) = triple_buffer([0.0_f64; 64]);

// Producer thread: fill the back slot, then publish it. Never blocks.
std::thread::spawn(move || {
    writer.write(|rows| rows[0] = 101.25);
    writer.publish();
});

// UI thread, once per frame: switch to the newest snapshot, if any.
if reader.update() {
    reader.read(|rows| draw(rows));
}
```

## Examples

<p align="center">
  <img alt="A fleet monitor: a live grid of hosts sorted by p99 latency beside a chart of the selected host's latency, with an incident's spike, over its traffic" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/monitor.png" width="660">
</p>
<p align="center">
  <img alt="The widget gallery: a toolbar with a filter input and buttons, a scrolling list beside a details pane in a splitter, and a status bar" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/gallery.png" width="660">
</p>
<p align="center">
  <img alt="A docking workspace: a live watchlist grid with prices flashing green and red as they change, beside a details pane above log and notes tabs" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/dock.png" width="660">
</p>
<p align="center">
  <img alt="A live chart: one-minute candles with a moving average over volume bars, with time labels below and prices on the right" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/chart.png" width="660">
</p>
<p align="center">
  <img alt="A grid of live instruments beside a chart of the selected instrument's trades: a price line over trade sizes" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/linked.png" width="660">
</p>
<p align="center">
  <img alt="A live table of instruments sorted by percent change" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/live-table.png" width="260">
  &nbsp;
  <img alt="A million-row Arrow table of orders" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/arrow-table.png" width="380">
</p>
<p align="center">
  <img alt="Wrapped, justified, centered and right-aligned paragraphs with a selection" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/paragraphs.png" width="560">
</p>

| Example | Shows |
|---|---|
| [`densa-widgets` `monitor`](crates/densa-widgets/examples/monitor.rs) | 1,000 simulated hosts in a live grid, slowest first, and a chart of the selected host's latency and traffic from a live samples table |
| [`densa-widgets` `gallery`](crates/densa-widgets/examples/gallery.rs) | A toolbar, a filterable list in a scroll view beside a details pane in a splitter, Tab between controls; `--light` for the light theme |
| [`densa-widgets` `dock`](crates/densa-widgets/examples/dock.rs) | Panels in tab groups (drag tabs to join or split groups, the gaps to resize) around a live watchlist grid: click a title to sort, a row to select; `--demo` starts mid-drag |
| [`densa-widgets` `live_table`](crates/densa-widgets/examples/live_table.rs) | 100,000 instruments in a live table, updated every frame: click a title to sort, type to filter, select a row and it follows its instrument |
| [`densa-widgets` `chart`](crates/densa-widgets/examples/chart.rs) | Live one-minute candles with a moving average over volume: scroll to zoom, drag to pan, double-click to follow the newest candle |
| [`densa-widgets` `linked`](crates/densa-widgets/examples/linked.rs) | A grid of 200 live instruments and a chart of the selected one's trades: a view of a live trades table, filtered by symbol and sorted by time |
| [`densa-arrow` `live`](crates/densa-arrow/examples/live.rs) | A live table updated continuously, its "top movers" view kept sorted |
| [`densa-arrow` `table`](crates/densa-arrow/examples/table.rs) | A million-row Arrow table; `--live` appends batches while it is shown |
| [`densa-paragraph` `paragraphs`](crates/densa-paragraph/examples/paragraphs.rs) | Wrapping, alignment, selection with mouse and keyboard |
| [`densa-paragraph` `editor`](crates/densa-paragraph/examples/editor.rs) | Editable text with input methods |
| [`trading-demo`](examples/trading-demo) | A streaming market grid: a pinned worker thread, a triple buffer and the grid |

## Guarantees and how they are checked

| Guarantee | Checked by |
|---|---|
| Live views and aggregates equal a full recomputation | Model-based tests check every view and every group after each of thousands of random upserts, deletes and compactions |
| Drawing, hit-testing, event delivery and selection allocate nothing | A counting global allocator in each crate's `tests/zero_alloc.rs` |
| The triple buffer is free of data races | loom model checking; a mutation script proves every weaker memory ordering fails |
| Correct pixels | GPU tests render offscreen and read the pixels back (on machines with a GPU) |
| Sound `unsafe` code | A `// SAFETY:` comment on every block (enforced by Clippy); Miri on the concurrency, allocator and platform crates |
| Tests that catch bugs | Deliberate bugs (mutants) must make the tests fail |

## Platform support

| | Linux | macOS | Windows |
|---|---|---|---|
| Rendering (wgpu) | Vulkan, OpenGL | Metal | Vulkan, DirectX 12, OpenGL |
| Text, paragraphs, editing | yes | yes | yes |
| Live tables, grid, triple buffer | yes | yes | yes |
| Widgets, layout, windows | yes | yes | yes |
| Thread placement and priority | not yet | not yet | yes |

A window draws with the best API the system offers unless the application picks one
(`WindowOptions::with_backend`); the `WGPU_BACKEND` environment variable (`vulkan`, `dx12`,
`metal`, `gl`) overrides both. Graphics drivers differ in what each API costs, memory most of
all, so measure on the machines you target.

## Limitations

- **Widgets:** labels, buttons, text inputs, scroll views, splitters, panels, docking, menus
  and a data grid, with flexbox layout, events, focus and accessibility. Not there yet: undo,
  redo and the clipboard in text inputs, keyboard shortcuts handled for menus (a menu shows
  them; the application handles the keys), and text inputs' caret and selection for screen
  readers. The grid selects one row at a time (kept on its record when the model gives
  rows keys), with no multiple selection, cell selection or editing yet. Docking does not yet
  save and restore layouts (panels already carry stable keys for it) or float panels in
  their own windows.
- **Charts:** lines, candlesticks and bars on number and time axes (UTC: no time zones yet),
  in stacked panes, fed from your data or a live table's view. Not there yet: logarithmic
  axes, area fills, annotations and drawing tools, several y axes in one pane, and candles
  built from a live table's trades (the app aggregates them). A line is one pixel column
  wide where it is steep, however thick it is set.
- **Live data:** views sort and filter, groups count, sum, average and find extremes; time
  windows and joins are not there yet. A table is used from one thread at a time.
- **Text:** grid cells are single-line, unshaped text in the bundled font. Word breaks in
  Chinese and Japanese and line breaks in Thai, Lao, Khmer and Burmese need densa-paragraph's
  `complex-scripts` feature (several MB of dictionaries). Not supported yet: choosing a
  language per paragraph (it picks the regional forms of Han characters), inline boxes,
  variable-font axes, synthetic bold and italic, color emoji.
- **Threads:** placement and priority are implemented on Windows only.

The [roadmap](https://densa.dev/roadmap/) lists what comes next.

## Development

```sh
cargo test --workspace                       # includes GPU readback and a short window run
cargo clippy --workspace --all-targets -- -D warnings
RUSTFLAGS="--cfg loom" cargo test -p densa-sync --release --test loom_triple_buffer
cargo +nightly miri test --target x86_64-unknown-linux-gnu -p densa-sync -p densa-alloc -p densa-platform
cargo +1.90 check --workspace --all-targets  # minimum supported Rust version
cargo bench -p densa-bench
```

Set `DENSA_SKIP_GPU_TESTS=1` on machines without a GPU or desktop session. Miri runs against
the Linux target: with a custom global allocator, Miri reports a false positive in the Windows
`System` allocator.

Bug reports and pull requests are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) explains how
the workspace is laid out, what the code must guarantee and every check a pull request has to
pass.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above,
without any additional terms or conditions.

The bundled font `crates/densa-text/fonts/JetBrainsMono-Regular.ttf` (JetBrains Mono 2.304)
is licensed separately under the SIL Open Font License 1.1; see
[crates/densa-text/fonts/OFL.txt](crates/densa-text/fonts/OFL.txt).
