# trading-demo

An application built on densa: a synthetic market feed streams into a table of instruments on
a worker thread, and the visible rows are drawn in a grid at the display's refresh rate.
Neither thread blocks the other, and neither allocates after startup. It is an example, not a
published crate.

<p align="center"><img alt="A data grid of 50 instruments with bid, ask, last and volume columns; changed prices flash green or red" src="https://raw.githubusercontent.com/densa-rs/densa/main/docs/screenshot.png" width="300"></p>

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

## How it works

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
  copies the rows on screen, and the UI thread tells it which rows through an atomic.
- **Both triple-buffer swaps use `AcqRel`**, which covers the reader releasing a slot the
  writer reuses. Every weaker ordering fails the loom model.
- **Rows are stored as structs, not columns.** A tick touches one row: one or two cache lines
  instead of one per column.
- **The worker runs on its own performance core**, at time-critical priority on Windows.
  On hybrid CPUs an unpinned worker migrates between cores and slows down on efficiency
  cores. Automatic placement never takes the machine's last free core, and a worker without
  a core of its own runs at normal priority, so the UI always has a core.
- **Text is rasterized once per glyph** into the atlas and drawn as textured quads; the demo
  prepares its character set at startup.
- **Cells flash briefly.** A changed price highlights its cell for 300 ms; with every
  instrument changing many times a second, whole-row highlights would never fade.

## Reading the metrics

Every second the demo prints a line of metrics (the window title shows a summary):

- **snapshot age**: time from the worker publishing data to a frame showing it. In
  rate-limited mode it is bounded by the batch interval; a smaller `--batch` lowers it.
- **tick age**: time since the newest tick among the visible rows: how fresh the data is.
- **heap ops**: heap operations since startup, per thread; 0 for the worker and the painter.
- **pacer resets**: times the worker fell far behind schedule (it was preempted).
  **migrations**: times the OS moved the worker to another CPU.

The demo's data model (`trading_demo::data`) and worker (`trading_demo::engine`) are
application code, not part of the published crates.
