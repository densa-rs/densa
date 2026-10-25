# Changelog

Every `densa-*` crate is released together under one version number. A crate can move to a new
version without changes of its own.

## Unreleased

### Added

- **Tables too large to hold** (`densa-arrow`, `densa-grid`, `densa-widgets`): `PagedModel`
  is a model for `DataGrid` whose rows your `PageSource` gives a page at a time (a query
  engine's result, a database table), on a thread of the model's own, so a source may take
  as long as it needs and the window never waits; rows not fetched yet show empty until they
  arrive. The grid fetches only the pages it shows and one on each side, those side by side
  in one call (each page alone if that call fails, so a page the source cannot give takes
  none beside it along), and skips those scrolled past before their turn. A click on a
  column title asks the source for its rows in that order (`PageQuery`, `SortKey`; a source
  can decline a column with `PageSource::can_sort`), without counting them again, even while
  the first count is on its way; `PageQuery::order_by_sql` writes that order as an SQL
  `ORDER BY`, with key columns breaking ties, for a source that pages with `LIMIT` and
  `OFFSET`. Pages far from the rows shown are dropped (`PageOptions`).
  `error` says why rows are missing (`FetchError`): a count or a call that failed or
  panicked, asked again on `retry`; rows that came back fewer than counted, which `refresh`
  counts and fetches again (as for any data that changed, showing the rows held until the new
  ones arrive); or the model's thread stopped, because the wake it was given panicked.
  `GridModel::showing` tells a model which rows the grid paints, and `GridModel::copy_rows`
  which rows a copy holds, so copying a billion selected rows copies only those fetched (both
  do what they did before by default); `DataGrid::copy` leaves the clipboard alone when the
  model offers none of the selected rows. The `paged` example shows a billion rows, computed
  when asked.

### Fixed

- **Charts of a few bars or candles** (`densa-widgets`): a `Chart` whose model gives an
  `x_step` zoomed in no further than five steps even when the data held fewer, so three bars
  took the right three of five places. The narrowest view is now all of the data when it
  holds fewer than five steps, and a view following the newest data keeps half a step of
  room after it on a number or time axis: bars on a category axis show edge to edge, and
  candles whole, the newest too. A wheel notch in no longer widens a view that data grown
  since left narrower than the narrowest. `ChartModel::x_extent` now says what a category
  axis's extent covers, the edges of the places (`-0.5` to `n - 0.5`): a model giving the
  first and last places' middles instead has its end bars cut when zoomed all the way in.

## 0.2.4

Nothing in the public API of 0.2.3 was removed or changed in a way that breaks code using it
(checked with `cargo-semver-checks`): projects asking for `0.2` get this release with `cargo update`.

### Added

- **Lookup joins** (`densa-data`): `LookupJoin` gives each row of a live table the columns of
  another table's row whose key it names (a trade its instrument, an order its customer): a
  left outer join kept up to date both ways. Rows going in on the left are joined as they
  come; a changed or deleted right row joins again every row naming it. The output is an
  ordinary `LiveTable` (`LookupJoin::table`), so views, groups, windows and grids work on it;
  rows removed from it (deleted, or out of a window) leave the join too. Refused with
  `DataError::JoinKey` when the `on` column cannot hold the right key's values, and with
  `DataError::JoinTable` when given a table that is not the join's output. `AGENTS.md` has a
  recipe joining a live table into a grid, and the `densa` crate's task map lists the join.

## 0.2.3

Nothing in the public API of 0.2.2 was removed or changed in a way that breaks code using it
(checked with `cargo-semver-checks`): projects asking for `0.2` get this release with `cargo update`.

### Added

- **Aggregates per time bucket** (`densa-data`): `GroupSpec::group_by_time(column, every)`
  groups rows by buckets of a time column (a minute, an hour), back to back from the
  column's zero, and `GroupSpec::group_by_step(column, step)` by buckets of a number column.
  The key column holds each bucket's start, in the column's type: counts, sums, minimums,
  maximums and means per minute, kept up to date as rows change. With a `Window` on the same
  column, the oldest bucket goes once its rows expire. A width that does not fit the column
  is refused with `DataError::BucketWidth`.
- **Docs for people and AI agents**: every crate's front page on docs.rs has an example that
  CI runs, and the `densa` crate's front page maps tasks to modules and shows a complete
  program (a live table in a grid, in a window). `AGENTS.md` at the repository's root gives
  coding agents the versions to use, how the pieces fit and which example to copy, with lines
  for your own project's `AGENTS.md`, and recipes (a menu bar, a grid model of your own,
  filtering as the user types, a chart following a live table, testing without a window);
  densa.dev serves the docs as plain text at `/llms.txt` and `/llms-full.txt`. `View`,
  `Groups`, `LiveTable::delete` and `GridSource::cell` have examples or notes on reading
  results, and the `monitor` example gets its data from a worker thread that wakes the window,
  the pattern applications should copy. These were found by agents new to densa doing tasks
  with it.

### Changed

- **Error messages say what to change**: `DataError` (unknown and unreadable columns, sums of
  text, window spans, bucket widths, duplicate names, batches of another schema, keys of the
  wrong kind), `SeriesError::NotNumeric`, `ParagraphError::InvalidSize` and
  `UiError::NoSuchWidget` now name what is accepted or how to fix the call. Only the text
  changed: the variants are the same.

## 0.2.2

Nothing in the public API of 0.2.1 was removed or changed in a way that breaks code using it
(checked with `cargo-semver-checks`): projects asking for `0.2` get this release with `cargo update`.

### Added

- **Files dropped on the window** (`densa-window`): `App::file_drop` is told of each file
  dragged over the window (`FileDrop::Hovering`), dragged away (`FileDrop::Left`) or dropped
  (`FileDrop::Dropped`). It does nothing by default. `WindowOptions::with_file_drops(false)`
  turns drops off (on Windows the window then leaves OLE alone, for apps whose thread uses
  COM in its multithreaded mode).
- **Category axes** (`densa-chart`, `densa-widgets`): `AxisKind::Category` (`Axis::category`)
  places named things at `0, 1, 2, ...`, such as the bars of a bar chart, with ticks only at
  whole places. `ChartModel::category` names each place; the `Chart` widget writes the names
  under the bars (every one, or every few when they are dense, cut short with `…` when long)
  and in the crosshair's box. `ChartPainter::paint_axes_named` and
  `ChartPainter::paint_crosshair_named` do the same for charts drawn without the widget.

## 0.2.1

Nothing in the public API of 0.2.0 was removed or changed in a way that breaks code using it
(checked with `cargo-semver-checks`): projects asking for `0.2` get this release with `cargo update`.

### Added

- **Time windows** (`densa-data`): `LiveTable::set_window` keeps a table to the rows whose
  time is within a span of the newest. `Window::duration` is for date, time, timestamp and
  duration columns; `Window::span` is for number columns, in their own units. Older rows are
  deleted as newer ones are upserted, counted in `Changes::deleted`, and views and groups
  follow. The window ends at the newest time in the table, not at the clock. Rows without a
  time (null or NaN) are kept. New error: `DataError::WindowSpan`.

### Changed

- **Number columns widen** (`densa-arrow`): a number or time column sized by the first rows
  widens when later values no longer fit (a count or a row number that grows), instead of
  showing `###`. It never narrows. Each later batch is measured on a sample of its rows that
  includes its last row; columns given a width with `set_column_chars` keep it.
- **Dragged widths stay** (`densa-widgets`): when a `DataGrid`'s model reports new column
  widths with the same column titles, the widths the user dragged are kept.

## 0.2.0

Nothing in the public API of 0.1.0 was removed or changed in a way that breaks code using it
(checked with `cargo-semver-checks`): this release adds.

### Added

- **Menus and popups** (`densa-ui`, `densa-widgets`, `densa-window`)
  - **Popup layer:** `Ui::open_popup` and `EventCx::open_popup` show a widget over the
    interface, placed by an `Anchor` (below, beside or at a point) and kept inside the window.
  - **Focus and keys:** a popup takes focus and gives it back when it closes. Tab stays inside
    it, and keys it ignores go to the widget that opened it.
  - **Closing:** a press or scroll outside closes the popups. The window closes them when it
    loses focus, changes size or changes scale.
  - **Actions:** `ActionKind::ContextMenu { at }` reports a right-click, the context menu key
    or Shift+F10. `ActionKind::Command(u32)` reports a chosen command.
  - **Menus:** `MenuItem` (commands with shortcut text, submenus, separators, check marks,
    unavailable items), `Menu` (pointer and keyboard navigation, jump by letter) and `MenuBar`.
  - **Accessibility:** menu, menu bar and menu item roles, with checked and expanded states.
- **Clipboard, undo and redo** (`densa-ui`, `densa-paragraph`, `densa-widgets`, `densa-window`)
  - **Clipboard:** the `Clipboard` trait, held by the `Ui` (`Ui::clipboard`,
    `EventCx::clipboard`). `MemoryClipboard` is the default; `SystemClipboard` in
    `densa-window` uses the system's (text only).
  - **Undo in `TextEditor`:** up to 100 steps. Typing is one step per word, a paste
    (`TextEditor::paste`) is a step of its own, and an input method's text becomes a step once
    committed.
  - **`TextInput`:** cut, copy, paste, undo and redo with the usual keys. The same actions are
    public methods, for Edit menus. Pasted line breaks become spaces.
  - **`DataGrid` ranges:** selects ranges of rows (Shift with the selection keys or a click,
    Ctrl+A), kept on their rows through updates and sorting. It copies them as tab-separated
    text (`DataGrid::copy`, `selected_text`, Ctrl+C), and a right-click selects the row under
    it.
- **Saved dock layouts** (`densa-widgets`)
  - **Saving:** `DockArea::layout` returns a `DockLayout`: splits and tab groups that name
    panels by their key.
  - **Restoring:** `DockArea::restore_layout` (or `set_layout` during an event) puts panels
    back by key. Keys without a panel are skipped, and panels the layout does not name join
    the first group.
  - **As text:** a layout writes itself as one line of versioned text and parses back
    (`Display`, `FromStr`). Errors say where and why (`DockLayoutError`).
- **Choosing the graphics API** (`densa-window`): `WindowOptions::with_backend(Backend::..)`.
  The `WGPU_BACKEND` environment variable still overrides it. A chosen API that gives no
  device falls back to the others.
- **Snapshots of live tables** (`densa-data`): `LiveTable::snapshot` returns the live rows as
  record batches in the table's schema, for SQL engines, file writers or other threads. Stored
  batches are shared where every row is live, not copied.
- **Pane titles** (`densa-chart`, `densa-widgets`): `ChartModel::pane_title` names a pane.
  The `Chart` widget writes the title on a line above the pane (`ChartPainter::title_height`,
  `ChartPainter::paint_title`).

### Changed

- **Dependency:** `densa-window` depends on `arboard` for the system clipboard.
