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

Rust libraries for GPU-rendered interfaces that show a lot of live data: trading terminals,
monitoring consoles, dashboards, developer tools. densa draws large, fast-changing tables and
text at the display's refresh rate, hands data between threads without locks, and renders
every frame without allocating memory.

<p align="center">
  <img alt="A data grid of 50 instruments with bid, ask, last and volume columns; changed prices flash green or red" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/screenshot.png" width="300">
  &nbsp;
  <img alt="Wrapped, justified, centered and right-aligned paragraphs with the word parley selected" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/paragraphs.png" width="480">
</p>

> **Status: 0.x, experimental.** APIs will change between releases, and the crates are not on
> crates.io yet. Builds and tests on Linux, macOS and Windows.

## Highlights

- **No allocation per frame.** Drawing, hit-testing and moving a selection allocate nothing
  once fonts and glyphs are loaded, and tests prove it with a counting allocator.
- **No locks between threads.** A wait-free triple buffer hands the newest data from a
  producer thread to the UI thread; its memory orderings are model-checked with loom.
- **One draw call per frame** for every rectangle and glyph.
- **Real text.** Shaping, wrapping, bidirectional text, alignment, selection, editing and
  input methods (IME), built on [parley](https://github.com/linebender/parley).
- **Bounded memory.** The glyph atlas grows to a fixed limit, then reuses the least recently
  used space, never a glyph drawn in the current frame.
- **Apache Arrow native.** Show record batches from Parquet, DataFusion, DuckDB or a stream
  directly: arrays are shared, not copied, and a million-row table draws a frame in ~52 µs.
- **Use only what you need.** Rendering, text and data hand-off are independent crates, and no
  third-party types appear in densa's APIs.

## Quick start

You need Rust 1.90 or later and a GPU with Vulkan, DX12 or Metal.

```sh
git clone https://github.com/densa-rs/densa
cd densa
cargo run --release -p trading-demo                          # live data grid
cargo run --release -p densa-paragraph --example paragraphs  # text and selection
cargo run --release -p densa-paragraph --example editor      # editable text
cargo run --release -p densa-arrow --example table -- --live  # a million-row Arrow table
```

To use the crates before they are published, depend on the repository:

```toml
[dependencies]
densa-draw = { git = "https://github.com/densa-rs/densa" }
densa-grid = { git = "https://github.com/densa-rs/densa" }
densa-render = { git = "https://github.com/densa-rs/densa" }
densa-arrow = { git = "https://github.com/densa-rs/densa" }
densa-paragraph = { git = "https://github.com/densa-rs/densa" }
densa-sync = { git = "https://github.com/densa-rs/densa" }
```

## Crates

| Crate | Use it for |
|---|---|
| [`densa-grid`](crates/densa-grid) | A data grid you fill from your own data (`GridSource`): column layout, alignment, cell styles, allocation-free painting |
| [`densa-draw`](crates/densa-draw) | A frame's display list: a `Canvas` that batches rectangles and glyphs into instances, independent of any GPU API |
| [`densa-render`](crates/densa-render) | Drawing with wgpu: a frame's instances in one draw call, with a glyph atlas that grows on the GPU |
| [`densa-paragraph`](crates/densa-paragraph) | Paragraphs: shaping, wrapping, bidirectional text, alignment, colored and sized spans, click-to-caret, selection, keyboard navigation, and editing with input methods (IME) |
| [`densa-arrow`](crates/densa-arrow) | Apache Arrow record batches in the data grid: typed, allocation-free cell formatting, appending batches while shown |
| [`densa-text`](crates/densa-text) | Fonts, glyph rasterization and a glyph atlas that grows with your text and fits the GPU's limits |
| [`densa-core`](crates/densa-core) | Shared primitives: `Color`, `Point`, `Rect` |
| [`densa-sync`](crates/densa-sync) | Wait-free triple buffer: hand the latest data from a producer thread to the UI thread without locks |
| [`densa-platform`](crates/densa-platform) | CPU topology, thread pinning and thread priority (Windows) |
| [`densa-fmt`](crates/densa-fmt) | Number formatting into stack buffers, without allocating |
| [`densa-alloc`](crates/densa-alloc) | A counting global allocator to prove a code path allocates nothing |

Only `densa-render` depends on a GPU or windowing crate: grids, Arrow tables, text and
paragraphs build a GPU-free display list, testable without a GPU.

## Usage

### A grid over your own data

Implement `GridSource` and the grid asks for the visible cells each frame. Cells are
formatted into a stack buffer, so painting allocates nothing.

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
[`examples/trading-demo/src/market.rs`](examples/trading-demo/src/market.rs) is a complete
source with formatting and flashing cells.

### Apache Arrow tables

```rust
use densa_arrow::ArrowTable;
use densa_grid::{GridPainter, Palette};

let mut table = ArrowTable::from_batches(batches)?; // Vec<RecordBatch>, shared not copied
table.set_decimals(3, Some(2))?; // fixed decimals for a float column
let mut painter = GridPainter::new(table.columns(), 40, font, 13.0, Palette::new());

// Every frame: a view scrolled to the first visible row. No allocation.
painter.paint(&table.view(first_row, &palette), &mut canvas);

// Streaming: append batches while the table is shown.
table.push(next_batch)?;
```

Columns are titled by field name, sized from the first rows and aligned by type. Integers,
floats, decimals, strings (including dictionaries and views), booleans, dates, times,
timestamps and durations are formatted without allocating; other types show `?`, and nulls
show a dimmed `null`. The [`table` example](crates/densa-arrow/examples/table.rs) scrolls a
million-row table and, with `--live`, appends a batch every 100 ms.

<p align="center"><img alt="A million-row Arrow table of orders: id, customer, region, unit price, total, time placed and paid flag" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/arrow-table.png" width="560"></p>

### Paragraphs

```rust
use densa_paragraph::{ParagraphStyle, TextAlign, TextSystem};
use densa_draw::{Canvas, Color, InstanceBuffer, Point};
use densa_text::{AtlasConfig, GlyphCache};

// Once per application: fonts, layout scratch space, the glyph atlas.
let mut text = TextSystem::new()?;
let mut glyphs = GlyphCache::new(AtlasConfig::default());
let mut instances = InstanceBuffer::with_capacity(8_192);

// When the text changes: shape, wrap and align.
let style = ParagraphStyle::new(15.0)
    .with_color(Color::srgb(0xC9D1D9))
    .with_align(TextAlign::Justify)
    .with_max_width(Some(480.0));
let mut paragraph = text.paragraph("Shaped once, wrapped to any width.", &style, &[])?;
paragraph.reflow(Some(320.0))?; // new width: re-wrap without reshaping

// Every frame: no allocation.
let mut canvas = Canvas::begin(&mut glyphs, &mut instances);
for run in paragraph.runs() {
    canvas.draw_run(run, Point::new(24.0, 24.0));
}
```

`TextSelection` turns clicks into caret positions and gives caret and selection rectangles to
draw. It moves by character, word and line: in screen order for arrow keys, in text order for
deletion, so right-to-left text behaves correctly. The
[`paragraphs` example](crates/densa-paragraph/examples/paragraphs.rs) supports click, drag,
shift+click, double- and triple-click, shift/ctrl + arrows and Home/End; resize the window to
re-wrap.

### Editable text

```rust
use densa_paragraph::{Motion, ParagraphStyle, TextEditor, TextSystem};

let mut text = TextSystem::new()?;
let mut editor = TextEditor::new(&mut text, "Hello", ParagraphStyle::new(15.0))?;
editor.insert(&mut text, ", brave world")?; // typed, pasted or committed by an input method
editor.delete(&mut text, Motion::PreviousWord)?; // ctrl+Backspace
editor.move_caret(Motion::TextStart, false); // moving the caret never allocates
```

`TextEditor` keeps the text, its layout and the selection in step. Backspace removes one
character (a combining accent comes off its letter; an emoji or line break goes whole), and
`delete` takes any motion: by word, to the line start, forward. For input methods, pass
winit's `Ime::Preedit` to `set_composition` and `Ime::Commit` to `insert`, underline
`for_each_composition_rect`, and give `ime_area` to `Window::set_ime_cursor_area` so the
candidate window opens next to the text. The
[`editor` example](crates/densa-paragraph/examples/editor.rs) does all of this.

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

## Guarantees and how they are checked

| Guarantee | Checked by |
|---|---|
| Drawing, hit-testing and selection allocate nothing | A counting global allocator in each crate's `tests/zero_alloc.rs` |
| The triple buffer is free of data races | loom model checking; a mutation script proves every weaker memory ordering fails |
| Correct pixels | GPU tests render offscreen and read the pixels back, including atlas growth and eviction (on machines with a GPU) |
| Sound `unsafe` code | A `// SAFETY:` comment on every block (enforced by Clippy); Miri on the concurrency, allocator and platform crates |
| Supported toolchain | CI checks the build with Rust 1.90 and runs the tests on Linux, macOS and Windows |

## Example: a real-time trading grid

`trading-demo` (in [`examples/`](examples/trading-demo)) streams a synthetic feed of 200,000
ticks per second into a 10,000-instrument table on a worker thread and renders the visible rows
at the display's refresh rate. Neither thread blocks the other, and neither allocates after
startup.

```sh
cargo run --release -p trading-demo                      # window
cargo run --release -p trading-demo -- --headless        # no window, prints metrics
cargo run --release -p trading-demo -- --headless --unbounded --batch 1024
cargo run --release -p trading-demo -- --list-cpus       # CPU topology and --pin choice
cargo run --release -p trading-demo -- --help
```

Window controls: mouse wheel, Up/Down, PageUp/PageDown, Home/End scroll; Esc quits.

| Option | Default | Meaning |
|---|---|---|
| `--headless` | off | Run the same frame loop without a window |
| `--seconds N` | 10 headless, until closed in a window | Run time |
| `--rate N` / `--unbounded` | 200000 | Target ticks per second, or as fast as possible |
| `--symbols N` | 10000 | Number of instruments |
| `--batch N` | 64 | Ticks per worker iteration |
| `--publish-interval-us N` | 100 | Minimum time between snapshot publishes |
| `--fps N` | 120 | Frame rate of the headless loop |
| `--pin auto\|none\|N` | auto | Worker placement |
| `--priority normal\|above\|high\|critical` | critical on Windows, normal elsewhere | Worker thread priority (Windows only) |
| `--seed N` | 1 | Seed of the synthetic feed |

### Performance

Intel i7-14700KF (8 performance + 12 efficiency cores), RTX 4060 Ti (Vulkan), 1920×1080 at
99 Hz, Windows 11, release build, 10,000 instruments, default settings.

| Run | Throughput | Frames | Data to screen p50 / p99 | Allocations after startup |
|---|---|---|---|---|
| Window, 200k ticks/s, V-Sync | 200.0k ticks/s | 99.8 fps (display rate) | 164 µs / 328 µs | 0 |
| Headless, 200k ticks/s | 200.0k ticks/s | 120.0 fps | 156 µs / 328 µs | 0 |
| Headless, unbounded, batch 1024 | 115.3M ticks/s | 120.0 fps | 47 µs / 107 µs | 0 |

Building a 50-row frame takes ~37 µs (p50) and ~66 µs (p99) on a performance core. Triple
buffer: publish 4.4 ns, read 10.6 ns. Glyph cache: 2,500 lookups in ~20 µs. A 50-row frame
of a million-row Arrow table (6 columns, scrolled to a new place each frame): ~52 µs. wgpu allocates
internally during submit and present; the allocation counts cover densa's code. Benchmarks are
pinned to performance cores: unpinned runs on hybrid CPUs vary by 2-3×.

### Reading the metrics

Every second the demo prints a line like this (the window title shows a summary):

```text
[  3s] 100 fps | paint p50 37.1us p99 65.5us | frame interval p50 10.49ms ... | snapshot age p50 155.6us p99 319.2us ...
```

- **snapshot age**: time from the worker publishing data to a frame showing it. In
  rate-limited mode it is bounded by the batch interval (64 ticks at 200k/s = 320 µs); a
  smaller `--batch` lowers it.
- **tick age**: time since the newest tick among the visible rows: how fresh the data is. Each
  of 10,000 instruments updates ~20 times per second.
- **heap ops**: heap operations since startup, per thread; 0 for the worker and the painter.
- **pacer resets**: times the worker fell over 10 ms behind schedule (it was preempted).
  **migrations**: times the OS moved the worker to another CPU.

### How it works

```text
 worker thread (pinned)                             UI thread
 ┌──────────────────────────────┐                   ┌─────────────────────────────┐
 │ feed  ─▶  table (one row per  │   triple buffer   │ read latest snapshot        │
 │           instrument)         │ ─────────────────▶│ GridPainter ─▶ Canvas       │
 │ copy visible rows, publish    │  visible rows     │ GpuRenderer: one draw call  │
 │                               │◀──── viewport ────│ (scrolling sets viewport)   │
 └──────────────────────────────┘                   └─────────────────────────────┘
```

- **Only the visible rows cross threads.** The worker keeps the whole table; each publish
  copies the ~50 rows on screen (~3 KB instead of ~480 KB), and the UI thread tells it which
  rows through an atomic.
- **Both triple-buffer swaps use `AcqRel`**, which covers the reader releasing a slot the
  writer reuses. Every weaker ordering fails the loom model.
- **Rows are stored as structs, not columns.** A tick touches one row: one or two cache lines
  instead of one per column. At 1M instruments this runs 39M ticks/s against 23M.
- **The worker is pinned to its own performance core**, at time-critical priority on Windows.
  Pinned, it runs 105-116M ticks/s; unpinned on a hybrid CPU it migrates thousands of times
  and drops to 30-55M on efficiency cores. Next to a busy neighbor on the same core, only
  time-critical priority keeps latency p99 under 1 ms. Automatic placement never takes the
  machine's last free core, and a worker without a core of its own runs at normal priority, so
  the UI always has a core.
- **Text is rasterized once per glyph** (at 4 subpixel positions) into the atlas and drawn as
  textured quads; the demo prepares its character set at startup.
- **Cells flash briefly.** A changed price highlights its cell for 300 ms; at ~20 changes per
  second per instrument, longer or whole-row highlights would never fade.

The demo's data model (`trading_demo::data`) and worker (`trading_demo::engine`) are
application code, not part of the published crates: every `densa-*` crate is a library, and
`examples/` holds applications.

## Platform support

| | Linux | macOS | Windows |
|---|---|---|---|
| Rendering (wgpu) | Vulkan | Metal | Vulkan, DX12 |
| Text, paragraphs, selection | yes | yes | yes |
| Triple buffer, counting allocator | yes | yes | yes |
| Thread pinning and priority | not yet | not yet | yes |

## Limitations

- **Widgets:** a data grid, selectable paragraphs and editable text. No layout system, event
  routing, focus handling or accessibility yet; text editing has no undo, redo or clipboard
  yet (applications can call `selected_text` and `insert`).
- **Text:** grid cells are single-line, unshaped text. Paragraphs use only the bundled font
  unless the `system-fonts` feature is enabled (no fallback for CJK without it). Not supported
  yet: inline boxes, variable-font axes, synthetic bold and italic, color emoji.
- **Glyphs:** if one frame needs more glyphs than the whole atlas holds, the extra ones appear
  in a later frame; glyphs larger than an atlas page are not drawn.
- **Threads:** pinning is not isolation. Next to a busy neighbor, a pinned, time-critical
  worker still stalls about twice per 6 s (max ~30 ms). CPU claims are tracked per process,
  and the UI thread is not pinned.
- **The demo's feed is synthetic.** No network ingestion, order book or persistence.

## Development

```sh
cargo test --workspace                       # includes GPU readback and a 2 s window run
cargo clippy --workspace --all-targets -- -D warnings
RUSTFLAGS="--cfg loom" cargo test -p densa-sync --release --test loom_triple_buffer
cargo +nightly miri test --target x86_64-unknown-linux-gnu -p densa-sync -p densa-alloc -p densa-platform
cargo +1.90 check --workspace --all-targets  # minimum supported Rust version
cargo bench -p densa-bench
```

Set `DENSA_SKIP_GPU_TESTS=1` on machines without a GPU or desktop session. Miri runs against
the Linux target: with a custom global allocator, Miri reports a false positive in the Windows
`System` allocator.

Bug reports and pull requests are welcome. Please run the checks above before opening a pull
request.

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
