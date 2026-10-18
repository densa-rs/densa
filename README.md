# densa

A Qt-like application framework in Rust: GPU rendering, real text and low-latency data
plumbing, built as reusable libraries. densa is designed for data-dense, real-time interfaces,
such as trading terminals, monitoring consoles, dashboards and developer tools, while no core
crate knows about any of those domains.

> **Status: early and experimental (0.x).** The foundations work and are measured; there are
> no widgets beyond a data grid and selectable paragraphs yet. APIs will change. Tested on
> Windows 11; CI builds and tests on Linux, macOS and Windows. Homepage: <https://densa.dev>.

![A trading grid built on densa: 50 of 10,000 live instruments at the display's refresh rate](https://raw.githubusercontent.com/densa-rs/densa/main/docs/screenshot.png)

## What is there

| Layer | Crate | What it gives you |
|---|---|---|
| Foundation | `densa-core` | Dependency-free primitives every crate shares: `Color`, `Point`, `Rect` |
| Text | `densa-text` | Fonts, glyph rasterization, a growable glyph atlas that frames read without allocating |
| Text | `densa-paragraph` | Paragraph layout on [parley](https://github.com/linebender/parley): shaping, wrapping, bidi, alignment, styled spans, hit-testing and selection |
| Rendering | `densa-render` | wgpu renderer: a `Canvas` every painter draws through, one instanced draw call, a generic data grid (`GridSource`) |
| Runtime | `densa-sync` | Wait-free SPSC triple buffer, model-checked with loom |
| Runtime | `densa-alloc` | Counting global allocator that proves "this frame allocates nothing" |
| Runtime | `densa-platform` | CPU topology, thread pinning and priority (Windows) |
| Runtime | `densa-fmt` | Allocation-free number formatting into stack buffers |

Dependencies only point downward, and layout never depends on rendering: a paragraph hands
the renderer plain glyph runs. Core crates never depend on GPU or windowing crates, and no
third-party type (parley, wgpu, skrifa) appears in densa's APIs.

### Principles

- **Frames do not allocate.** Laying out text or loading fonts may; drawing, hit-testing and
  moving a selection may not. Every such claim is a test against a counting allocator.
- **Measured, not claimed.** Benchmarks are pinned to performance cores; GPU output is checked
  by reading pixels back; tests are checked by deliberately breaking the code (mutants) and
  watching them fail.
- **Libraries, not app code.** Domains (markets, logs, metrics) live in applications; the
  framework offers generic building blocks such as the `GridSource` trait.

## Text

`densa-paragraph` shapes, wraps and aligns text and draws it through the same glyph atlas as
the grid. Resizing re-breaks lines without reshaping; drawing a laid-out paragraph allocates
nothing. `TextSelection` hit-tests points, moves by character, word and line (visual order for
arrow keys, logical order for deletion), and gives caret and selection rectangles, also
without allocating.

![Wrapped, justified, centered and right-aligned paragraphs with Vietnamese text and a selection](https://raw.githubusercontent.com/densa-rs/densa/main/docs/paragraphs.png)

```sh
cargo run --release -p densa-paragraph --example paragraphs
```

In the example: click, drag, shift+click, double- and triple-click, shift/ctrl + arrows,
Home/End; resize the window to watch the paragraphs re-wrap.

The glyph atlas grows a page at a time and then evicts the least recently used glyphs, never
one drawn in the current frame; it caps itself to the GPU's texture limit, and an atlas the
GPU can no longer mirror is reported as an error rather than drawn wrong.

## Demo: a real-time trading grid

`densa-poc` is an application built on densa, and the first stress test of its foundations: a
worker thread applies a synthetic feed of 200,000+ ticks/s to a 10,000-instrument table and
hands the visible rows to the render thread through the wait-free triple buffer, while the
grid renders at the display's refresh rate. Neither thread ever blocks on the other, and after
startup neither allocates, verified by the counting allocator on every run.

### Results

Intel i7-14700KF (8 P-cores + 12 E-cores), RTX 4060 Ti (Vulkan), 1920x1080 @ 99 Hz,
Windows 11, release build, 10,000 symbols, worker pinned to a P-core at time-critical
priority (the defaults). Numbers are from the summary line each run prints.

| Run | Throughput | Frames | Snapshot age p50 / p99 | Heap ops after startup |
|---|---|---|---|---|
| Window, 200k ticks/s target, V-Sync | 200.0k ticks/s | 99.8 fps (display rate) | 164 us / 328 us | worker 0, paint 0 |
| Headless, 200k ticks/s target | 200.0k ticks/s | 120.0 fps | 156 us / 328 us | worker 0, frame loop 0 |
| Headless, unbounded, batch 1024 | 115.3M ticks/s | 120.0 fps | 47 us / 107 us | worker 0, frame loop 0 |

Building a frame's 50 rows of instances (`GridPainter::paint`) takes ~37 us (p50) and ~66 us
(p99) with real text (JetBrains Mono 13px). That is on a P-core: the render thread is not
pinned, and when Windows schedules it on an E-core the same work takes ~70 us (p50). wgpu
itself allocates during submit/present (~199 heap operations per frame); that is measured and
reported separately, not hidden.

Micro-benchmarks (criterion, bench thread pinned to a P-core): triple buffer publish 4.4 ns,
update + read 10.6 ns, 2.8 KB fill + publish with a reader spinning on another core 72 ns;
`snapshot_into` (50 rows) 51 ns; a warm glyph-cache frame of 2,500 lookups ~20 us.

### Running it

Requirements: Rust 1.90+, a GPU with Vulkan, DX12 or Metal for the window.

```sh
cargo run --release -p densa-poc                         # window (default)
cargo run --release -p densa-poc -- --headless           # no window, prints metrics
cargo run --release -p densa-poc -- --headless --unbounded --batch 1024
cargo run --release -p densa-poc -- --list-cpus          # CPU topology and --pin choice
cargo run --release -p densa-poc -- --help
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
| `--pin auto\|none\|N` | auto | Worker placement (see below) |
| `--priority normal\|above\|high\|critical` | critical on Windows, normal elsewhere | Worker thread priority (Windows only; see below) |
| `--seed N` | 1 | Seed of the synthetic feed |

Every second a line like this is printed (and the window title shows a summary):

```text
[  3s] 100 fps | paint p50 37.1us p99 65.5us | frame interval p50 10.49ms ... | snapshot age p50 155.6us p99 319.2us ...
```

- **snapshot age**: time from the worker publishing a snapshot to a frame displaying it. This
  is the pipeline latency. In rate-limited mode it is bounded by the batch interval (64 ticks at
  200k/s = 320 us); a smaller `--batch` lowers it.
- **tick age**: time since the newest tick among the visible rows. This measures how *fresh*
  the data is, not the pipeline: each of 10,000 symbols updates only ~20 times per second.
- **heap ops**: heap operations since startup, per thread. The worker and the frame painting
  must stay at 0. "gpu submit" is wgpu's internal work.
- **pacer resets**: times the worker fell over 10 ms behind schedule and restarted it, i.e. it
  was preempted. **migrations**: times the OS moved the worker to another CPU.

### Architecture

```text
 worker thread (pinned P-core)                      render thread (main)
 ┌──────────────────────────────┐                   ┌─────────────────────────────┐
 │ TickGenerator  ─▶ MarketTable │   triple buffer   │ Reader::update / read       │
 │  (wyrand walk)     (AoS rows) │ ─────────────────▶│ FrameProbe (latency)        │
 │        snapshot_into(viewport)│  50-row snapshot  │ GridPainter ─▶ Canvas       │
 │        Writer::publish        │◀──── viewport ────│ GpuGrid: 1 instanced draw   │
 └──────────────────────────────┘   (AtomicU32 hint) └─────────────────────────────┘
```

The demo's own crates: `densa-data` (`MarketTable`, `TickGenerator`, `ViewportSnapshot<N>`),
`densa-engine` (ingest worker, pacer, latency histogram, frame probe) and `densa-poc` (market
columns, argument parsing, window and headless modes). They are application code and are not
published; generic parts of `densa-engine` will move into the framework as other data sources
arrive.

### Design decisions, and the evidence behind them

- **Both triple-buffer swaps use `AcqRel`.** "Release on publish, Acquire on fetch" is unsound:
  it misses the write-after-read edge when the writer reuses a slot the reader just released.
  `scripts/ordering_mutants.ps1` weakens each ordering and checks that loom reports a causality
  violation for every weaker variant.
- **Snapshots carry only the visible viewport.** The worker keeps the whole table privately and
  publishes 50 rows (~3 KB); the render thread tells it which rows through an atomic hint.
  Copying the full table per publish would be ~480 KB.
- **Array-of-structs, not structure-of-arrays.** Each tick updates one random row: one or two
  cache lines in AoS, one per column (seven) in SoA. Pinned benchmarks at 1M symbols: 39M ticks/s
  vs 23M for SoA; equal when the table fits in cache.
- **Pin the worker to a performance core.** On this hybrid CPU an unpinned busy-spinning worker
  migrated 446-4,442 times per 5 s and ran at 30-55M ticks/s whenever it landed on an E-core,
  versus 105-116M pinned to a P-core. A process-wide registry gives every auto-pinned worker its
  own physical core.
- **Pinned *and* time-critical.** Pinning decides where the worker runs, not who wins the
  core: next to a busy normal-priority process pinned to the same core
  (`scripts/priority_experiment.ps1`), a normal-priority worker reached 100.8k of 200k ticks/s
  with snapshot age p99 32.5 ms; `above`/`high` only reached ~170k with p99 ~30 ms; only
  `critical` held 198k with p99 under 1 ms. The demo defaults to it on Windows (`--priority
  normal` opts out); thread priorities are not implemented on other systems yet, so there the
  default is `normal`. The library default stays `Normal`.
- **One draw call.** Every quad is a solid rectangle or a glyph from a coverage atlas, drawn
  as instances. Glyphs are rasterized on first use (4 subpixel positions) and uploaded once;
  the demo rasterizes its whole character set at startup, so frames never allocate. Building
  instances is GPU-free code with unit tests; the GPU half is verified by rendering offscreen
  and reading pixels back.
- **The renderer is generic.** Apps describe columns and implement `GridSource`, formatting
  each cell into a stack buffer; markets, prices and flashes live in the demo, not the renderer.
- **Only the LAST cell flashes, for 300 ms.** With each symbol changing ~20 times per second, a
  first version that flashed whole rows for 600 ms turned the grid into a wall of color.

## Verification

```sh
cargo test --workspace                       # includes GPU readback and a 2 s window run
cargo clippy --workspace --all-targets -- -D warnings
RUSTFLAGS="--cfg loom" cargo test -p densa-sync --release --test loom_triple_buffer
cargo +nightly miri test --target x86_64-unknown-linux-gnu -p densa-sync -p densa-alloc -p densa-platform
cargo llvm-cov --workspace --summary-only    # ~94% of lines
cargo +1.90 check --workspace --all-targets  # MSRV
cargo bench -p densa-bench
powershell -File scripts/ordering_mutants.ps1
powershell -File scripts/pinning_experiment.ps1
powershell -File scripts/priority_experiment.ps1
```

Set `DENSA_SKIP_GPU_TESTS=1` on machines without a GPU or desktop session (CI does).

Miri runs against the Linux target on purpose: with a custom global allocator it executes the
real Windows `System` allocator, whose over-aligned path reports UB under Miri even with a plain
`static GLOBAL: System = System;`. Miri cannot model the Windows FFI in `densa-platform`; those
calls are covered by tests that watch threads actually move between CPUs.

## Limitations

- **Few widgets.** A data grid and selectable paragraphs; no text input, layout system,
  event routing or accessibility yet.
- **Text:** grid cells are unshaped, single-line text (one glyph per character, `?` for missing
  ones); characters outside the prewarmed set allocate once, the first time they are drawn.
  Paragraphs are shaped, wrapped and bidi-ordered, but by default use only the bundled font (no
  fallback: CJK shows missing-glyph boxes; the `system-fonts` feature adds the OS fonts). Not
  yet: inline boxes, variable-font axes, synthetic bold/italic, color emoji, text editing
  (insertion, deletion, IME).
- **The glyph atlas is bounded** (by default four 1024 x 1024 pages, 4 MiB, capped to the GPU's
  texture limit). If a single frame needs more glyphs than the whole atlas holds, the extra
  ones are skipped that frame (counted) and placed in a later frame; glyphs larger than a page
  are never drawn.
- **Threads:** CPU pinning and priority are Windows-only (elsewhere `--pin auto` runs unpinned
  and says so); the CPU claim registry is per process; and pinning is not isolation: a
  time-critical pinned worker next to a busy neighbor still stalled about twice per 6 s (max
  ~30 ms), plausibly Windows' boosting of starved threads. Reserving a core is the remaining
  step. The render thread is not pinned.
- **The demo's feed is synthetic.** No network ingestion, order book or persistence.
- wgpu allocates per frame internally; "zero-allocation frames" covers densa's own code.

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
