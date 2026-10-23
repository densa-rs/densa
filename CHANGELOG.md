# Changelog

Every `densa-*` crate is released together under one version number. A crate can move to a new
version without changes of its own.

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
