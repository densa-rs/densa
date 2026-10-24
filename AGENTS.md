# AGENTS.md

Notes for AI coding agents (and the people working with them) on densa: first for building an
application with it, then for changing densa itself. The API documentation on
[docs.rs](https://docs.rs/densa) is the reference; [densa.dev/llms.txt](https://densa.dev/llms.txt)
lists the same material as plain text.

## Versions: check these before writing code

densa is younger than most training data. Use what the installed crates say, not memory:

- densa 0.2 from crates.io, all `densa-*` crates at the same version. Rust 1.90 or later,
  edition 2021.
- Arrow 60 (`arrow-array`, `arrow-schema`). Build record batches with the versions densa
  re-exports, `densa_data::arrow_array` and `densa_data::arrow_schema` (also
  `densa::data::arrow_array`): a different Arrow version gives errors such as "expected
  `RecordBatch`, found `RecordBatch`".
- wgpu 30 and winit 0.30, used only inside `densa-render` and `densa-window`. Applications
  built on `densa_window::run` never touch them.
- No async runtime. Data arrives on threads of your own; the window thread owns the interface.

## Building an application

Start from the `densa` crate (features `window` and `data`, both on by default) or depend on
the crates you use. Its front page on docs.rs maps tasks to modules and has a complete
program: a live table shown in a grid, in a window.

Complete programs to copy from, in `crates/densa-widgets/examples/`:

| Example | Shows |
|---|---|
| `monitor.rs` | a live table in a grid, sorted, and a chart of the selected row's history |
| `linked.rs` | a grid and a chart over two live tables, the chart filtered by the selected row |
| `live_table.rs` | 100,000 rows, 1,000 updated every frame; click a title to sort, type to filter |
| `dock.rs` | panels in tabs and splits, dragged around; with `--layout FILE` the layout is saved and restored |
| `gallery.rs` | the standard widgets: menus, a toolbar, a list, a splitter, text input |
| `chart.rs` | a chart of your own data with panes, zoom and pan |

Run one with `cargo run --release -p densa-widgets --example monitor`.

How the pieces fit:

- `densa_window::run(WindowOptions, app)` opens the window and calls your `App`: `build`
  once (add widgets under `ui.root()`), `action` for each action a widget reports (a click,
  an edit, a selected row), `frame` before each paint (take in new data; return `true` to keep
  painting continuously).
- Widgets live in the `Ui` and are reached by `WidgetId`. Change one with
  `ui.update::<Type, _>(id, |widget, cx| ...)`; read one with `ui.widget::<Type>(id)`. Do not
  keep references to widgets.
- Layout is flexbox (`densa_ui::Style`) in logical pixels; painting is in physical pixels.
- Live data: a `densa_data::LiveTable` keyed by an integer or text column. `upsert` replaces
  rows by key; views (`ViewSpec`: sort and filter) and grouped aggregates (`GroupSpec`) follow
  every change; `Window` keeps only recent rows. Every batch must have the table's columns:
  the same names, order and data types (`LiveTable::schema`). Show a table with `DataGrid<densa_arrow::LiveModel>`, and change
  it through `grid.model_mut(cx).table_mut()`.
- Other threads never touch the `Ui`. They hand data over (a channel, or
  `densa_sync::triple_buffer` for snapshots) and call `Waker::wake` (from `host.waker()`) so a
  frame comes.
- A frame must not allocate once its text is cached. Format numbers with `densa_fmt` into
  stack buffers, not `format!`.

Errors are typed (`DataError`, `UiError`, `ParagraphError`, `WindowError`, ...) and their
messages say what to change: read the message before guessing.

## Changing densa

[CONTRIBUTING.md](CONTRIBUTING.md) is the full guide: the workspace layout, what the code must
guarantee, and every check a pull request must pass. In short:

- Libraries, not app code: no core crate knows about a domain (markets, hosts, sensors);
  applications bring theirs through traits such as `GridSource`, `GridModel` and
  `ChartModel`.
- Dependencies point downward only (app, window, widgets, ui, draw and paragraph, text,
  core). Only `densa-render` and `densa-window` touch the GPU or a window.
- Frames do not allocate. Prove it with the counting allocator in the crate's
  `tests/zero_alloc.rs`, which must `assert!(is_active())` first.
- Every `unsafe` block has a `// SAFETY:` comment, and every atomic ordering a comment
  saying why it is enough.
- Tests are named as sentences and check behaviour, including model-based tests that compare
  against a simple reference. A change is checked by planting bugs in it: a test that still
  passes with a deliberate bug is missing something.
- Public docs, error messages and examples are plain English, give a runnable example, and
  say how to fix what went wrong.

Before calling a change done:

```sh
cargo fmt --all
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo +1.90 check --workspace --all-targets
```

Set `DENSA_SKIP_GPU_TESTS=1` where there is no GPU or desktop session.

## In your own project

Add this to your project's `AGENTS.md` (or `CLAUDE.md`) so an agent finds densa's docs:

```md
## densa
This project uses densa 0.2 (https://densa.dev). Before writing densa code, read
https://densa.dev/llms.txt and the `densa` crate's front page on docs.rs; copy from
densa-widgets' examples. Use the Arrow types densa re-exports (`densa_data::arrow_array`).
Widgets change only through `ui.update::<T, _>(id, ...)` on the window thread; other threads
send data and call `Waker::wake`. Frames must not allocate: format with `densa_fmt`.
```
