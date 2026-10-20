//! Selecting rows, one or a range, and copying them.

use std::ops::Range;

use densa_grid::{CellText, GridModel, Palette};
use densa_ui::{ActionKind, EventCx};

use super::DataGrid;

impl<M: GridModel> DataGrid<M> {
    /// The selected rows: one, or the range from where a Shift selection started to the
    /// [selected](Self::selected) row; empty for none.
    pub fn selected_rows(&self) -> Range<usize> {
        let Some(focus) = self.selected() else {
            return 0..0;
        };
        let anchor = self.anchor.map_or(focus, |a| a.row);
        anchor.min(focus)..anchor.max(focus) + 1
    }

    /// Selects the rows from `anchor` to `row` (either way round), `row` being the
    /// [selected](Self::selected) one, and scrolls it into view. Rows past the last are
    /// taken as the last.
    pub fn select_rows(&mut self, cx: &mut EventCx<'_>, anchor: usize, row: usize) {
        let Some(last) = self.model.rows().checked_sub(1) else {
            return;
        };
        self.select(cx, Some(row.min(last)));
        self.anchor = Some(self.selection_of(anchor.min(last)));
    }

    /// Selects every row, the first being the [selected](Self::selected) one, without
    /// scrolling.
    pub fn select_all(&mut self, cx: &mut EventCx<'_>) {
        let Some(last) = self.model.rows().checked_sub(1) else {
            return;
        };
        if self.selected() != Some(0) {
            self.selected = Some(self.selection_of(0));
            cx.emit(ActionKind::Selected(0));
        }
        self.anchor = Some(self.selection_of(last));
        cx.request_paint();
    }

    /// Puts the selected rows on the clipboard, as [`selected_text`](Self::selected_text)
    /// writes them; `false` if no row is selected or the clipboard would not take them.
    pub fn copy(&mut self, cx: &mut EventCx<'_>) -> bool {
        let rows = self.selected_rows();
        if rows.is_empty() {
            return false;
        }
        let text = self.selected_text(&super::palette(cx.theme()));
        cx.clipboard().set_text(&text).is_ok()
    }

    /// The selected rows' cells as the grid shows them (with `palette`'s formatting, where
    /// the model's text depends on it), a tab between cells and a line break between rows:
    /// the shape spreadsheets paste into cells. Tabs and line breaks within a cell become
    /// spaces.
    pub fn selected_text(&mut self, palette: &Palette) -> String {
        self.model.prepare();
        let columns = self.model.columns().len();
        let mut out = String::new();
        let mut cell = CellText::new();
        for (index, row) in self.selected_rows().enumerate() {
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

    /// Selects `row` alone (reporting it if it was not selected) and shows it.
    pub(super) fn select_row(&mut self, cx: &mut EventCx<'_>, row: usize) {
        self.anchor = None;
        self.move_selection(cx, row);
    }

    /// Selects from where the selection started (the selected row, if it was one row) to
    /// `row`: Shift with a click or a key.
    pub(super) fn extend_to(&mut self, cx: &mut EventCx<'_>, row: usize) {
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
