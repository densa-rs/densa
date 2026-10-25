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
  `~/.cargo/git/checkouts/densa-<hash>/<commit>/` (`densa-` and 16 hex digits, not
  `densa-connectors-...`), where `<commit>` is the first 7 characters of the hash after `#`
  in the `densa` entries of your `Cargo.lock`. Older commits sit beside it: read the one
  your build uses.
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
| `monitor.rs` | a live table in a grid, sorted, and a chart of the selected row's history, fed by a worker thread that wakes the window (the pattern to copy for live data) |
| `linked.rs` | a grid and a chart over two live tables, the chart filtered by the selected row |
| `live_table.rs` | 100,000 rows, 1,000 updated every frame (made inside `frame`, a stress test: copy `monitor.rs` for real data); click a title to sort, type to filter |
| `paged.rs` | a billion rows, none held: a `PagedModel` fetches the pages on screen from a `PageSource` (the pattern for a query engine or a database); click a title to sort at the source |
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
- A table too large to hold (a query engine's result, a database table):
  `DataGrid<densa_arrow::PagedModel>` with your `densa_arrow::PageSource`, which counts the
  rows and gives a page (`offset`, `limit`, in the order of a `PageQuery`) on a thread of the
  model's own. The grid fetches only the pages it shows; see the recipe below.
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

### Recipes

Each compiles as written (with `use` lines for the names it takes from `densa::`).

A menu bar whose commands reach the app:

```rust
use densa::ui::ActionKind;
use densa::widgets::{MenuBar, MenuItem};

const OPEN: u32 = 1;
const QUIT: u32 = 2;
let file = [
    MenuItem::new("Open…", OPEN).with_shortcut("Ctrl+O"), // the shortcut is shown; keys are yours
    MenuItem::separator(),
    MenuItem::new("Quit", QUIT),
];
let bar = ui.add(ui.root(), MenuBar::new().with_menu("File", file), MenuBar::style())?;
// In App::action: a chosen item arrives as ActionKind::Command(OPEN) from a menu.
let chosen = |kind: ActionKind| match kind {
    ActionKind::Command(command) => Some(command),
    _ => None,
};
```

A grid over data of your own: implement `densa::grid::GridModel` and give it to
`DataGrid::new`. The types it needs are all in `densa::grid`:

```rust
use densa::grid::{Align, CellStyle, CellText, Column, GridModel, Palette, SortOrder};

struct Orders {
    columns: Vec<Column>, // Column::new("Item", 16, Align::Left), ...
    rows: Vec<(u64, String)>, // (id, item)
}

impl GridModel for Orders {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn rows(&self) -> usize {
        self.rows.len()
    }

    fn cell(&self, row: usize, _column: usize, text: &mut CellText, palette: &Palette) -> CellStyle {
        // CellText is a 64-byte stack buffer: push_str, or densa::fmt's writers for numbers.
        // Text that does not fit is refused: write a placeholder such as "#" instead.
        if text.push_str(&self.rows[row].1).is_err() {
            let _ = text.push_str("#");
        }
        CellStyle::new(palette.text)
    }

    // Optional: sorting on a title click, and keys so a selected row follows its record.
    fn sort(&mut self, _column: usize, order: SortOrder) -> bool {
        self.rows.sort_by(|a, b| a.1.cmp(&b.1));
        if order == SortOrder::Descending {
            self.rows.reverse();
        }
        true
    }

    fn row_key(&self, row: usize) -> Option<u64> {
        self.rows.get(row).map(|r| r.0)
    }

    fn row_of_key(&self, key: u64) -> Option<usize> {
        self.rows.iter().position(|r| r.0 == key)
    }
}
```

A live grid filtered as the user types: on `ActionKind::Changed` from a `TextInput`, give the
grid's `LiveModel` a filter (`Filter::contains` is case-sensitive):

```rust
use densa::arrow::LiveModel;
use densa::data::Filter;
use densa::ui::{Action, ActionKind, Ui, WidgetId};
use densa::widgets::{DataGrid, TextInput};

fn on_action(ui: &mut Ui, input: WidgetId, grid: WidgetId, action: &Action) {
    if action.source != input || action.kind != ActionKind::Changed {
        return;
    }
    let text = ui.widget::<TextInput>(input).map_or(String::new(), |t| t.text().to_owned());
    let filter = (!text.is_empty()).then(|| Filter::contains("symbol", text));
    ui.update::<DataGrid<LiveModel>, _>(grid, |grid, cx| {
        let _ = grid.model_mut(cx).set_filter(filter); // Err: no such column, or not text
    });
}
```

Rows enriched from another table (each trade with its instrument's columns), kept up to date
as either side changes: a `densa::data::LookupJoin`. Its output is a table of your own, so
it can sit in a grid:

```rust
use densa::arrow::LiveModel;
use densa::data::arrow_array::RecordBatch;
use densa::data::LookupJoin;
use densa::widgets::DataGrid;

fn enriched(
    ui: &mut densa::ui::Ui,
    trades: RecordBatch,      // id (key), symbol, price, ...
    instruments: RecordBatch, // symbol (key), name, ...
) -> Result<(LookupJoin, densa::ui::WidgetId), Box<dyn std::error::Error>> {
    let mut join = LookupJoin::new(trades.schema(), "id", "symbol", instruments.schema(), "symbol")?;
    let mut table = join.table()?;
    join.upsert_right(&mut table, instruments)?;
    join.upsert_left(&mut table, trades)?;
    let grid = DataGrid::new(LiveModel::new(table)?);
    let grid = ui.add(ui.root(), grid, DataGrid::<LiveModel>::style())?;
    // Later, on the window thread: hand the grid's table to the join.
    //   ui.update::<DataGrid<LiveModel>, _>(grid, |g, cx| join.upsert_left(g.model_mut(cx).table_mut(), more));
    Ok((join, grid))
}
```

A grid over a table too large to hold: implement `densa::arrow::PageSource` (its calls run on
a thread of the model's own, so they may wait on a database or a query engine; return the
batches as they come, the model joins them) and give it to a `PagedModel`, with a wake that
brings a frame when pages arrive. A copy of selected rows holds only the rows fetched.
`model.error()` says why rows are missing (a `FetchError`); `retry` asks again for what
failed, `refresh` counts and fetches again after the data changed:

```rust
use densa::arrow::{PageError, PageQuery, PageSource, PagedModel};
use densa::data::arrow_array::RecordBatch;
use densa::data::arrow_schema::SchemaRef;
use densa::widgets::DataGrid;

struct Trips {
    schema: SchemaRef, // every page must have these columns' data types
}

impl PageSource for Trips {
    fn row_count(&mut self, _query: &PageQuery) -> Result<u64, PageError> {
        Ok(2_000_000_000) // SELECT count(*) FROM trips
    }

    fn page(
        &mut self,
        query: &PageQuery,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, PageError> {
        // SELECT * FROM trips ORDER BY <key.column()> [DESC] LIMIT <limit> OFFSET <offset>
        for key in query.sort() {
            let _ = (key.column(), key.is_descending());
        }
        let _ = (offset, limit);
        Ok(vec![RecordBatch::new_empty(self.schema.clone())]) // the batches the engine gave
    }
}

// In App::build (`host` is its `&mut Host`):
let waker = host.waker();
let source = Trips { schema: schema.clone() };
let model = PagedModel::new(schema, source, move || {
    waker.wake();
})?;
let grid = ui.add(ui.root(), DataGrid::new(model), DataGrid::<PagedModel>::style())?;
```

A chart that follows a live table: keep the table, a view sorted by time, and a
`densa::arrow::LiveSeries` per line in your `ChartModel`; call `series.sync(&table)` after
every change to the table, or the chart shows nothing. `monitor.rs` (its `Trail` model) and
`linked.rs` are complete examples.

Typing in a test without a window: give the input focus, then send keys.

```rust
use densa::ui::{Key, KeyEvent, NamedKey};

ui.set_focus(Some(input));
ui.key(&KeyEvent::down(Key::Character("A")).with_text("A"));
ui.key(&KeyEvent::down(Key::Named(NamedKey::Backspace)));
let edits = ui.actions().filter(|a| a.source == input).count(); // a Changed per edit
```

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
