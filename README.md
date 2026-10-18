<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/densa-rs/densa/main/docs/brand/densa-logo-dark.svg">
    <img alt="densa" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/brand/densa-logo.svg" width="320">
  </picture>
</h1>

<p align="center"><b>Dense by design.</b></p>

<p align="center">
  <a href="https://github.com/densa-rs/densa/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/densa-rs/densa/actions/workflows/ci.yml/badge.svg"></a>
  <a href="#license"><img alt="License: MIT or Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-2F5BEA"></a>
  <img alt="Rust 1.90+" src="https://img.shields.io/badge/rust-1.90%2B-2F5BEA">
  <a href="https://densa.dev"><img alt="Website: densa.dev" src="https://img.shields.io/badge/web-densa.dev-0B1220"></a>
</p>

densa is a Rust toolkit for applications that show a lot of live data: trading terminals,
monitoring consoles, operational dashboards, developer tools. It keeps live data in order as
it changes, and draws it on the GPU at the display's refresh rate, from the data stream to
the pixel.

> **Status: 0.x, experimental.** APIs will change between releases, and the crates are not on
> crates.io yet. Builds and tests on Linux, macOS and Windows.

## Architecture

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/densa-rs/densa/main/docs/architecture-dark.svg">
    <img alt="densa's crates in three lanes: data (Arrow sources, densa-data, densa-arrow), interface (densa-grid, densa-widgets, densa-paragraph, densa-ui, densa-text, densa-draw, densa-render, densa-window, the GPU) and runtime (densa-sync, densa-platform, densa-fmt, densa-alloc), all on densa-core" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/architecture.svg" width="900">
  </picture>
</p>

- **Data.** Arrow record batches from files, streams or query engines go into live tables;
  views sort and filter them, groups aggregate them, and both follow every change
  incrementally.
- **Interface.** A tree of widgets, laid out with flexbox, takes pointer, keyboard and input
  method events; widgets (buttons, text inputs, scroll views, a data grid) draw into a
  display list that is independent of any GPU API, and `densa-window` presents a whole frame
  in one draw call with wgpu.
- **Runtime.** Hand data between threads without locks, place threads on the right cores,
  format numbers without allocating, and prove that frames never allocate.

Each crate is usable on its own, and only `densa-render` and `densa-window` depend on a GPU
or windowing crate.

## Highlights

- **Live tables.** Upsert Arrow batches by key; sorted, filtered views and grouped
  aggregates are kept up to date as rows change instead of being recomputed.
- **Widgets with flexbox layout.** A retained widget tree laid out with flexbox, with pointer
  capture, hover, focus and Tab order, and widgets for toolbars, forms, lists and split
  panes; it runs headless, so interfaces are tested without a window.
- **Arrow from end to end.** Batches are shared, never copied, from the source to the
  screen; a grid over millions of rows costs only its visible rows.
- **No allocation per frame.** Drawing, scrolling, hit-testing, delivering events and moving
  a caret allocate nothing once fonts and glyphs are loaded; tests prove it with a counting
  allocator.
- **No locks between threads.** A wait-free triple buffer hands the newest data to the UI
  thread; its memory orderings are model-checked with loom.
- **Real text.** Shaping, wrapping, bidirectional text, selection, editing and input methods
  (IME), built on [parley](https://github.com/linebender/parley).
- **One draw call per frame**, with a glyph atlas whose memory is bounded.

## Quick start

You need Rust 1.90 or later and a GPU with Vulkan, Metal or DirectX 12.

```sh
git clone https://github.com/densa-rs/densa
cd densa
cargo run --release -p densa-widgets --example gallery      # widgets in a window
cargo run --release -p densa-arrow --example live            # a live table, sorted as it moves
cargo run --release -p densa-arrow --example table           # a million-row Arrow table
cargo run --release -p densa-paragraph --example editor      # editable text
cargo run --release -p trading-demo                          # a streaming market grid
```

To use the crates before they are published, depend on the repository:

```toml
[dependencies]
densa-data = { git = "https://github.com/densa-rs/densa" }
densa-widgets = { git = "https://github.com/densa-rs/densa" }
densa-window = { git = "https://github.com/densa-rs/densa" }
```

## Crates

| Crate | Use it for |
|---|---|
| [`densa-data`](crates/densa-data) | Live Arrow tables: keyed upserts and deletes, sorted and filtered views and grouped aggregates kept up to date incrementally |
| [`densa-arrow`](crates/densa-arrow) | Arrow tables and live views in the data grid, with typed, allocation-free cell formatting |
| [`densa-ui`](crates/densa-ui) | The widget tree: flexbox layout, pointer, keyboard and input method events, focus, themes; no window or GPU needed |
| [`densa-widgets`](crates/densa-widgets) | Labels, buttons, text inputs, scroll views, splitters, panels |
| [`densa-window`](crates/densa-window) | Running an interface in a native window: winit input in, a wgpu frame out |
| [`densa-grid`](crates/densa-grid) | A data grid over your own data (`GridSource`): columns, alignment, cell styles |
| [`densa-paragraph`](crates/densa-paragraph) | Paragraphs: shaping, wrapping, bidirectional text, selection, keyboard navigation, editing with input methods |
| [`densa-text`](crates/densa-text) | Fonts, glyph rasterization and a glyph atlas that fits the GPU's limits |
| [`densa-draw`](crates/densa-draw) | A frame's display list: a `Canvas` that batches rectangles and glyphs, independent of any GPU API |
| [`densa-render`](crates/densa-render) | Drawing a display list with wgpu in one draw call |
| [`densa-sync`](crates/densa-sync) | A wait-free triple buffer: the latest data from a producer thread to the UI thread |
| [`densa-platform`](crates/densa-platform) | CPU topology, thread placement and priority (Windows today) |
| [`densa-fmt`](crates/densa-fmt) | Number formatting into stack buffers, without allocating |
| [`densa-alloc`](crates/densa-alloc) | A counting global allocator, to prove a code path allocates nothing |
| [`densa-core`](crates/densa-core) | Shared primitives: `Color`, `Point`, `Rect`, `Size` |

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

To show it, `densa_arrow::LiveGrid` turns a view into the grid's data source; the
[`live` example](crates/densa-arrow/examples/live.rs) does this with 100,000 instruments
updated continuously.

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
  <img alt="The widget gallery: a toolbar with a filter input and buttons, a scrolling list beside a details pane in a splitter, and a status bar" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/gallery.png" width="660">
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
| [`densa-widgets` `gallery`](crates/densa-widgets/examples/gallery.rs) | A toolbar, a filterable list in a scroll view beside a details pane in a splitter, Tab between controls; `--light` for the light theme |
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
| Rendering (wgpu) | Vulkan | Metal | Vulkan, DirectX 12 |
| Text, paragraphs, editing | yes | yes | yes |
| Live tables, grid, triple buffer | yes | yes | yes |
| Widgets, layout, windows | yes | yes | yes |
| Thread placement and priority | not yet | not yet | yes |

## Limitations

- **Widgets:** labels, buttons, text inputs, scroll views, splitters, panels and a data grid,
  with flexbox layout, events and focus. Not there yet: accessibility, docking, tabs, menus,
  undo, redo and the clipboard in text inputs; the grid is not yet a widget.
- **Live data:** views sort and filter, groups count, sum, average and find extremes; time
  windows and joins are not there yet. A table is used from one thread at a time.
- **Text:** grid cells are single-line, unshaped text. Paragraphs use only the bundled font
  unless the `system-fonts` feature is enabled. Not supported yet: inline boxes,
  variable-font axes, synthetic bold and italic, color emoji.
- **Threads:** placement and priority are implemented on Windows only.

The [roadmap](https://densa.dev/vision) lists what comes next.

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
