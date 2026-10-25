//! Selecting rows, one or a range, and copying them.

use std::ops::Range;

use densa_grid::{CellText, GridModel, Palette};
use densa_ui::{ActionKind, EventCx};

use super::DataGrid;

impl<M: GridModel> DataGrid<M> {
    /// The selected rows: one, the range from where a Shift selection started to the
    /// [selected](Self::selected) row, or all of them; empty for none.
    pub fn selected_rows(&self) -> Range<usize> {
        let rows = self.model.rows();
        if self.all {
            return 0..rows;
        }
        let Some(focus) = self.selected() else {
            return 0..0;
        };
        let anchor = self.anchor.map_or(focus, |a| a.row);
        // Until the next layout or paint finds its records again, a row can be past the end.
        let end = (anchor.max(focus) + 1).min(rows);
        anchor.min(focus).min(end)..end
    }

    /// Selects the rows from `anchor` to `row` (either way round), `row` being the
    /// [selected](Self::selected) one, and scrolls it into view. Rows past the last are
    /// taken as the last.
    pub fn select_rows(&mut self, cx: &mut EventCx<'_>, anchor: usize, row: usize) {
        let Some(last) = self.model.rows().checked_sub(1) else {
            return;
        };
        self.all = false;
        self.anchor = None;
        self.select(cx, Some(row.min(last)));
        self.anchor = Some(self.selection_of(anchor.min(last)));
    }

    /// Selects every row, and those added later, until the selection changes. The
    /// [selected](Self::selected) row stays as it was, and nothing scrolls.
    pub fn select_all(&mut self, cx: &mut EventCx<'_>) {
        self.all = true;
        cx.request_paint();
    }

    /// Puts the selected rows on the clipboard, as [`selected_text`](Self::selected_text)
    /// writes them; `false`, leaving the clipboard as it was, if no row is selected, the model
    /// offers none of them to copy ([`GridModel::copy_rows`]), or the clipboard would not take
    /// them.
    ///
    /// The text of every selected row the model offers (all of them, unless it fetches its
    /// rows) is built at once, on the calling thread: a million rows take a noticeable time and
    /// as much memory as their text.
    pub fn copy(&mut self, cx: &mut EventCx<'_>) -> bool {
        self.catch_up();
        let runs = self.copied_runs();
        if runs.is_empty() {
            return false;
        }
        let text = self.text_of(runs, &super::palette(cx.theme()));
        cx.clipboard().set_text(&text).is_ok()
    }

    /// The selected rows' cells as the grid shows them (with `palette`'s formatting, where
    /// the model's text depends on it), a tab between cells and a line break between rows:
    /// the shape spreadsheets paste into cells. Tabs and line breaks within a cell become
    /// spaces. Only the rows the model offers to copy are written
    /// ([`GridModel::copy_rows`]).
    pub fn selected_text(&mut self, palette: &Palette) -> String {
        self.catch_up();
        let runs = self.copied_runs();
        self.text_of(runs, palette)
    }

    /// The selected rows a copy holds, in runs: those the model offers, within the selection
    /// (a model narrows a copy, never widens it).
    fn copied_runs(&self) -> Vec<Range<usize>> {
        let selected = self.selected_rows();
        if selected.is_empty() {
            return Vec::new();
        }
        self.model
            .copy_rows(selected.clone())
            .into_iter()
            .map(|run| run.start.max(selected.start)..run.end.min(selected.end))
            .filter(|run| !run.is_empty())
            .collect()
    }

    /// The text of the rows of `runs`, as [`selected_text`](Self::selected_text) writes it.
    fn text_of(&self, runs: Vec<Range<usize>>, palette: &Palette) -> String {
        let columns = self.model.columns().len();
        let mut out = String::new();
        let mut cell = CellText::new();
        let rows = runs.into_iter().flatten();
        for (index, row) in rows.enumerate() {
            if index > 0 {
                out.push('\n');
            }
            for column in 0..columns {
                if column > 0 {
                    out.push('\t');
                }
                cell.clear();
                self.model.cell(row, column, &mut cell, palette);
                out.extend(cell.as_str().chars().map(|c| match c {
                    '\t' | '\n' | '\r' => ' ',
                    c => c,
                }));
            }
        }
        out
    }

    /// Takes in what the model changed since the last frame, as painting does.
    fn catch_up(&mut self) {
        self.model.prepare();
        self.follow_model();
        self.clamp();
    }

    /// Selects `row` alone (reporting it if it was not selected) and shows it.
    pub(super) fn select_row(&mut self, cx: &mut EventCx<'_>, row: usize) {
        self.anchor = None;
        self.all = false;
        self.move_selection(cx, row);
    }

    /// Selects from where the selection started (the selected row, if it was one row) to
    /// `row`: Shift with a click or a key.
    pub(super) fn extend_to(&mut self, cx: &mut EventCx<'_>, row: usize) {
        self.all = false;
        if self.anchor.is_none() {
            self.anchor = Some(self.selected.unwrap_or_else(|| self.selection_of(row)));
        }
        self.move_selection(cx, row);
    }

    /// Makes `row` the selected row, keeping the anchor, and shows it.
    fn move_selection(&mut self, cx: &mut EventCx<'_>, row: usize) {
        if self.selected() != Some(row) {
            self.selected = Some(self.selection_of(row));
            cx.emit(ActionKind::Selected(row));
        }
        self.reveal_row(row);
        cx.request_paint();
    }
}
