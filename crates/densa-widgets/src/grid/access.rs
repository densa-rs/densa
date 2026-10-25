//! The grid to assistive technology: a table of a header row and the rows on screen.

use densa_core::Rect;
use densa_grid::{CellText, GridModel, SortOrder};
use densa_ui::{AccessAction, AccessNode, AccessRequest, EventCx, Handled, PaintCx, Role};

use super::DataGrid;

/// Rows are numbered by their row, wrapping past this; only the rows on screen (far fewer)
/// are in the tree, so wrapped numbers never meet, and a row keeps its number while it shows.
const ROWS: u32 = 1 << 12;
/// The header row's part number.
const HEADER: u32 = ROWS;
/// The header cells' part numbers start here.
const HEADER_CELLS: u32 = HEADER + 1;
/// Columns a cell number makes room for; cells of later columns are left out.
const MAX_COLUMNS: u32 = 1 << 10;
/// Row cells' part numbers start here: a block of columns per row number.
const CELLS: u32 = HEADER_CELLS + MAX_COLUMNS;

/// The part number of row `row`.
fn row_part(row: usize) -> u32 {
    // Lossless: the remainder is below ROWS.
    (row % ROWS as usize) as u32
}

/// The part number of cell `column` of row `row` (`None` for the header row's).
fn cell_part(row: Option<usize>, column: usize) -> Option<u32> {
    let column = u32::try_from(column).ok().filter(|c| *c < MAX_COLUMNS)?;
    Some(match row {
        None => HEADER_CELLS + column,
        Some(row) => CELLS + row_part(row) * MAX_COLUMNS + column,
    })
}

/// The row number a row's or a cell's part number stands for.
fn part_row(part: u32) -> Option<u32> {
    if part < ROWS {
        return Some(part);
    }
    let cell = part.checked_sub(CELLS)?;
    let row = cell / MAX_COLUMNS;
    (row < ROWS).then_some(row)
}

impl<M: GridModel> DataGrid<M> {
    /// The rows on screen (each one painted, the last perhaps cut off): those with nodes,
    /// and those the model hears it shows ([`GridModel::showing`]).
    pub(super) fn shown_rows(&self) -> std::ops::Range<usize> {
        let shown = self
            .visible_rows()
            .min(self.model.rows().saturating_sub(self.first_row));
        self.first_row..self.first_row + shown
    }

    /// A table: the header row with a column header per column (naming the sort), then each
    /// row on screen with its cells; a row is selected when chosen, chosen by a click on it
    /// or a cell, and has focus while the grid does.
    pub(super) fn describe(&self, cx: &PaintCx<'_>, node: &mut AccessNode) {
        node.set_role(Role::Table);
        let (Some(frame), Some(layout)) = (self.frame(cx.rect()), self.layout_of()) else {
            return;
        };
        let width = frame.rect.width();
        let max_x = self.max_scroll_x(width);
        for (action, can) in [
            (AccessAction::ScrollUp, self.first_row > 0),
            (
                AccessAction::ScrollDown,
                self.first_row < self.last_first_row(),
            ),
            (AccessAction::ScrollLeft, self.scroll_x > 0.0),
            (AccessAction::ScrollRight, self.scroll_x < max_x),
        ] {
            if can {
                node.add_action(action);
            }
        }
        let count = self.columns.len().min(layout.column_count());
        let cell_rect = |column: usize, top: f32, bottom: f32| {
            let half = layout.gap() as f32 / 2.0;
            let left = frame.rect.left() + layout.column_x(column) as f32
                - half
                - self.shift_of(column, width);
            let right = left + layout.column_width(column) as f32 + 2.0 * half;
            Rect::from_edges(left, top, right, bottom)
        };

        let mut header = AccessNode::new(Role::Row);
        header.set_bounds(frame.header);
        for column in 0..count {
            let Some(part) = cell_part(None, column) else {
                break;
            };
            let mut cell = AccessNode::new(Role::ColumnHeader);
            let title = self.columns[column].title.as_ref();
            match self.sort_order() {
                Some((c, SortOrder::Ascending)) if c == column => {
                    cell.set_label(format!("{title}, sorted ascending"));
                }
                Some((c, _)) if c == column => {
                    cell.set_label(format!("{title}, sorted descending"));
                }
                _ => cell.set_label(title),
            }
            cell.set_bounds(cell_rect(column, frame.header.top(), frame.header.bottom()));
            header.push_part(part, cell);
        }
        node.push_part(HEADER, header);

        let row_height = layout.row_height() as f32;
        let palette = super::palette(cx.theme());
        let mut text = CellText::new();
        for (slot, row) in self.shown_rows().enumerate() {
            let top = frame.body.top() + slot as f32 * row_height;
            let bottom = (top + row_height).min(frame.body.bottom());
            let mut item = AccessNode::new(Role::Row);
            item.set_bounds(Rect::from_edges(
                frame.rect.left(),
                top,
                frame.rect.right(),
                bottom,
            ));
            item.set_selected(self.selected_rows().contains(&row));
            item.add_action(AccessAction::Click);
            item.add_action(AccessAction::ScrollIntoView);
            for column in 0..count {
                let Some(part) = cell_part(Some(row), column) else {
                    break;
                };
                text.clear();
                self.model.cell(row, column, &mut text, &palette);
                let mut cell = AccessNode::new(Role::Cell);
                cell.set_value(text.as_str());
                cell.set_bounds(cell_rect(column, top, bottom));
                cell.add_action(AccessAction::Click);
                item.push_part(part, cell);
            }
            node.push_part(row_part(row), item);
        }
        // The selected row, when on screen, is where the focus is.
        if let Some(row) = self.selected().filter(|r| self.shown_rows().contains(r)) {
            node.set_focused_part(row_part(row));
        }
    }

    /// Scrolls the table, or selects (a click) or shows (scroll into view) a row, asked of
    /// it or one of its cells.
    pub(super) fn act(&mut self, cx: &mut EventCx<'_>, request: &AccessRequest<'_>) -> Handled {
        let Some(part) = request.part else {
            return self.scroll_table(cx, request.action);
        };
        // Only rows on screen have nodes: the one numbered as the part.
        let Some(number) = part_row(part) else {
            return Handled::No;
        };
        let Some(row) = self.shown_rows().find(|r| row_part(*r) == number) else {
            return Handled::No;
        };
        match request.action {
            AccessAction::Click => {
                self.select_row(cx, row);
                Handled::Yes
            }
            AccessAction::ScrollIntoView => {
                self.reveal_row(row);
                cx.request_paint();
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    /// Scrolls a page up or down, or sideways by a column's worth.
    fn scroll_table(&mut self, cx: &mut EventCx<'_>, action: AccessAction) -> Handled {
        let page = self.whole_rows().max(1);
        let max_x = self.max_scroll_x(cx.rect().width());
        let sideways = cx.px(48.0);
        let before = (self.first_row, self.scroll_x);
        match action {
            AccessAction::ScrollUp => self.first_row = self.first_row.saturating_sub(page),
            AccessAction::ScrollDown => {
                self.first_row = self
                    .first_row
                    .saturating_add(page)
                    .min(self.last_first_row());
            }
            AccessAction::ScrollLeft => self.scroll_x = (self.scroll_x - sideways).max(0.0),
            AccessAction::ScrollRight => self.scroll_x = (self.scroll_x + sideways).min(max_x),
            _ => return Handled::No,
        }
        if (self.first_row, self.scroll_x) == before {
            return Handled::No;
        }
        cx.request_paint();
        Handled::Yes
    }
}
