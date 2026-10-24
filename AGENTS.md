# AGENTS.md

Notes for AI coding agents (and the people working with them) on densa: first for building an
application with it, then for changing densa itself. The API documentation on
[docs.rs](https://docs.rs/densa) is the reference; [densa.dev/llms.txt](https://densa.dev/llms.txt)
lists the same material as plain text.

## Reading the docs and the source

- Fetch these files as they are (`curl https://densa.dev/llms-full.txt`): a tool that
  summarizes a page drops the item names and signatures you need.
  [llms-full.txt](https://densa.dev/llms-full.txt) holds this file and every crate's front
  page, with an example each.
- docs.rs shows the last release. Code newer than it (on the main branch, listed under
  "Unreleased" in the CHANGELOG) is documented in the source.
- The source of the densa you build is on your disk, with every doc comment and example. From
  crates.io: `~/.cargo/registry/src/*/densa-data-0.2.*/` (one folder per crate). From git:
  `~/.cargo/git/checkouts/densa-*/<commit>/`, where `<commit>` is the first 7 characters of
  the hash after `#` in the `densa` entries of your `Cargo.lock` (older commits sit beside it).
- The examples in `crates/densa-widgets/examples/` and the tests in each crate's `tests/`
  folder show every widget in use.

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
- Widgets are in `densa::widgets` (`Label`, `Button`, `TextInput`, `Panel`, `ScrollView`,
  `Splitter`, `DockArea`, `MenuBar`, `DataGrid`, `Chart`). Add one with
  `ui.add(parent, widget, style)`: `parent` is `ui.root()` or a container such as `Panel`, and
  `style` a `densa::ui::Style`, usually `Style::row()` or `Style::column()` with `.gap(..)`,
  `.padding(..)` and `.grow(1.0)` (widgets that fill space, such as `DataGrid`, give theirs:
  `DataGrid::<M>::style()`). The root is a column the size of the window.
- Layout is flexbox in logical pixels; painting is in physical pixels.
- A widget reports what the user did as an `Action { source, kind }` (`ActionKind::Clicked`,
  `Changed`, `Submitted`, `Selected(row)`, ...), passed to `App::action`; read the details from
  the widget (`ui.widget::<TextInput>(id)?.text()`).
- Live data: a `densa_data::LiveTable` keyed by an integer or text column (the key column may
  be declared nullable; a row with a null key is refused). `upsert` replaces
  rows by key; views (`ViewSpec`: sort and filter) and grouped aggregates (`GroupSpec`) follow
  every change; `Window` keeps only recent rows. Every batch must have the table's columns:
  the same names, order and data types (`LiveTable::schema`). Delete by key with an Arrow array
  of the key column's type (`table.delete(&StringArray::from(vec!["h1"]))`). Read a view's
  rows through `table.view(id)`: each `view.row(i)` is a `RowRef` naming one of
  `table.batches()` and a row in it; read grouped results with
  `table.groups(id).unwrap().to_batch()` (keys first, then the aggregates in the order you added
  them). The rustdoc of `View` and `Groups` shows both. Show a table with
  `DataGrid<densa_arrow::LiveModel>`, and change it through `grid.model_mut(cx).table_mut()`.
- Other threads never touch the `Ui`. They hand data over (a channel, or
  `densa_sync::triple_buffer` for snapshots) and call `Waker::wake` (from `host.waker()`) so a
  frame comes.
- A frame must not allocate once its text is cached. Format numbers with `densa_fmt` into
  stack buffers, not `format!`.

Errors are typed (`DataError`, `UiError`, `ParagraphError`, `WindowError`, ...) and their
messages say what to change: read the message before guessing.

### Testing without a window

A `Ui` runs without a window or GPU, so widgets and whole forms can be tested with
`cargo test`:

```rust
use densa::core::{Point, Size};
use densa::paragraph::TextSystem;
use densa::ui::{ActionKind, PointerButton, PointerEvent, PointerKind, Style, Ui};
use densa::widgets::Button;

let mut ui = Ui::new(TextSystem::bundled()?); // the bundled font: the same on every machine
ui.resize(Size::new(800.0, 600.0));
let save = ui.add(ui.root(), Button::new("Save"), Style::row())?;
ui.layout();

let r = ui.rect(save).unwrap();
let at = Point::new((r.left() + r.right()) / 2.0, (r.top() + r.bottom()) / 2.0);
for kind in [PointerKind::Down(PointerButton::Primary), PointerKind::Up(PointerButton::Primary)] {
    ui.pointer(PointerEvent::new(kind, at));
}
let clicked = ui.actions().any(|a| a.source == save && a.kind == ActionKind::Clicked);
assert!(clicked);
```

`ui.paint(&mut canvas)` draws the frame into a `densa::draw::Canvas`, for tests that count
what was drawn.

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
