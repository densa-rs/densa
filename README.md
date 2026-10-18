<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/densa-rs/densa/main/docs/brand/densa-logo-dark.svg">
    <img alt="densa" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/brand/densa-logo.svg" width="320">
  </picture>
</h1>

<p align="center"><b>Dense by design.</b></p>

Rust libraries for GPU-rendered, real-time interfaces that show a lot of live data: trading
terminals, monitoring consoles, dashboards, developer tools. densa renders large, fast-changing
tables and text at the display's refresh rate, hands data between threads without locks, and
draws every frame without allocating memory.

> **Status: 0.x, experimental.** APIs will change between releases. Builds and tests on
> Linux, macOS and Windows. Homepage: <https://densa.dev>.

![A trading grid built with densa: 50 of 10,000 live instruments at the display's refresh rate](https://raw.githubusercontent.com/densa-rs/densa/main/docs/screenshot.png)

## Crates

| Crate | Use it for |
|---|---|
| `densa-core` | Shared primitives: `Color`, `Point`, `Rect` |
| `densa-render` | Drawing with wgpu: a `Canvas` that batches rectangles and text into one draw call, and a data grid you fill from your own data (`GridSource`) |
| `densa-text` | Fonts, glyph rasterization and a glyph atlas that grows with your text and fits the GPU's limits |
| `densa-paragraph` | Paragraphs: shaping, wrapping, bidirectional text, alignment, colored and sized spans, click-to-caret, selection and keyboard navigation |
| `densa-sync` | Wait-free triple buffer: hand the latest data from a producer thread to the UI thread without locks |
| `densa-alloc` | A counting global allocator to prove a code path allocates nothing |
| `densa-platform` | CPU topology, thread pinning and thread priority (Windows) |
| `densa-fmt` | Number formatting into stack buffers, without allocating |

Rendering, text layout and data hand-off are independent: use only the crates you need. No
GPU or windowing dependency outside `densa-render`, and no third-party types in densa's APIs.

## Guarantees

- **No allocation per frame.** Drawing the grid or a laid-out paragraph, hit-testing and moving
  a selection allocate nothing once fonts and glyphs are loaded. Each of these is a test
  against a counting allocator.
- **No locks between the data thread and the UI thread.** The triple buffer is wait-free on
  both sides and model-checked with loom.
- **One draw call per frame** for all rectangles and glyphs.
- **Bounded memory for glyphs.** The atlas grows a page at a time up to a limit (4 MiB by
  default, capped to the GPU's texture size), then reuses the space of the least recently used
  glyphs, never one drawn in the current frame.

## Text

```sh
cargo run --release -p densa-paragraph --example paragraphs
```

![Wrapped, justified, centered and right-aligned paragraphs with Vietnamese text and a selection](https://raw.githubusercontent.com/densa-rs/densa/main/docs/paragraphs.png)

Paragraphs are shaped with [parley](https://github.com/linebender/parley), wrapped to a width
and aligned (start, end, left, center, right, justify); changing the width re-wraps without
reshaping. `TextSelection` turns clicks into caret positions and gives caret and selection
rectangles to draw; it moves by character, word and line, in screen order for arrow keys and
in text order for deletion, so right-to-left text behaves correctly.

In the example: click, drag, shift+click, double- and triple-click, shift/ctrl + arrows,
Home/End; resize the window to re-wrap.

## Example: a real-time trading grid

`trading-demo` (in `examples/`) streams a synthetic feed of 200,000 ticks/s into a 10,000-instrument table on a
worker thread and renders the visible rows at the display's refresh rate. Neither thread
blocks the other, and neither allocates after startup.

```sh
cargo run --release -p trading-demo                      # window
cargo run --release -p trading-demo -- --headless        # no window, prints metrics
cargo run --release -p trading-demo -- --headless --unbounded --batch 1024
cargo run --release -p trading-demo -- --list-cpus       # CPU topology and --pin choice
cargo run --release -p trading-demo -- --help
```

Requirements: Rust 1.90+, a GPU with Vulkan, DX12 or Metal for the window. Window controls:
mouse wheel, Up/Down, PageUp/PageDown, Home/End scroll; Esc quits.

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

Intel i7-14700KF (8 P-cores + 12 E-cores), RTX 4060 Ti (Vulkan), 1920x1080 @ 99 Hz,
Windows 11, release build, 10,000 symbols, default settings.

| Run | Throughput | Frames | Data-to-screen latency p50 / p99 | Allocations after startup |
|---|---|---|---|---|
| Window, 200k ticks/s, V-Sync | 200.0k ticks/s | 99.8 fps (display rate) | 164 us / 328 us | 0 |
| Headless, 200k ticks/s | 200.0k ticks/s | 120.0 fps | 156 us / 328 us | 0 |
| Headless, unbounded, batch 1024 | 115.3M ticks/s | 120.0 fps | 47 us / 107 us | 0 |

Building a 50-row frame takes ~37 us (p50) and ~66 us (p99) on a performance core. Triple
buffer: publish 4.4 ns, read 10.6 ns. Glyph cache: 2,500 lookups ~20 us. (wgpu allocates
internally during submit and present; the allocation counts above cover densa's code.)

### Reading the metrics

Every second the demo prints a line like this (the window title shows a summary):

```text
[  3s] 100 fps | paint p50 37.1us p99 65.5us | frame interval p50 10.49ms ... | snapshot age p50 155.6us p99 319.2us ...
```

- **snapshot age**: time from the worker publishing data to a frame showing it. In
  rate-limited mode it is bounded by the batch interval (64 ticks at 200k/s = 320 us); a smaller
  `--batch` lowers it.
- **tick age**: time since the newest tick among the visible rows: how fresh the data is. Each
  of 10,000 symbols updates ~20 times per second.
- **heap ops**: heap operations since startup, per thread; 0 for the worker and the painter.
- **pacer resets**: times the worker fell over 10 ms behind schedule (it was preempted).
  **migrations**: times the OS moved the worker to another CPU.

## How it works

```text
 worker thread (pinned)                             UI thread
 ┌──────────────────────────────┐                   ┌─────────────────────────────┐
 │ feed  ─▶  table (one row per  │   triple buffer   │ read latest snapshot        │
 │           instrument)         │ ─────────────────▶│ GridPainter ─▶ Canvas       │
 │ copy visible rows, publish    │  visible rows     │ GpuGrid: one draw call      │
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

## Development

```sh
cargo test --workspace                       # includes GPU readback and a 2 s window run
cargo clippy --workspace --all-targets -- -D warnings
RUSTFLAGS="--cfg loom" cargo test -p densa-sync --release --test loom_triple_buffer
cargo +nightly miri test --target x86_64-unknown-linux-gnu -p densa-sync -p densa-alloc -p densa-platform
cargo +1.90 check --workspace --all-targets  # MSRV
cargo bench -p densa-bench
```

Set `DENSA_SKIP_GPU_TESTS=1` on machines without a GPU or desktop session. Miri runs against
the Linux target: with a custom global allocator, Miri reports a false positive in the Windows
`System` allocator.

## Limitations

- **Widgets:** a data grid and selectable paragraphs. No text input, layout system, event
  routing or accessibility yet.
- **Text:** grid cells are single-line, unshaped text. Paragraphs use only the bundled font
  unless the `system-fonts` feature is enabled (no fallback for CJK without it). Not supported
  yet: inline boxes, variable-font axes, synthetic bold/italic, color emoji, text editing and
  IME.
- **Glyphs:** if one frame needs more glyphs than the whole atlas holds, the extra ones appear
  in a later frame; glyphs larger than an atlas page are not drawn.
- **Threads:** pinning and priority are implemented on Windows only. Pinning is not
  isolation: next to a busy neighbor a pinned, time-critical worker still stalls about twice
  per 6 s (max ~30 ms). CPU claims are tracked per process. The UI thread is not pinned.
- **The demo's feed is synthetic.** No network ingestion, order book or persistence.

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
