//! The grid's input: the wheel, clicks on rows and titles, dragging column edges and the
//! scroll bar, and the selection keys.

use densa_core::Point;
use densa_grid::{GridModel, SortOrder};
use densa_ui::{
    ActionKind, Cursor, Event, EventCx, Handled, Key, KeyEvent, NamedKey, PointerButton,
    PointerEvent, PointerKind, ScrollDelta,
};

use super::paint::BAR_GRAB;
use super::{DataGrid, Drag};

/// Rows a wheel notch scrolls.
const WHEEL_ROWS: f32 = 3.0;
/// Logical pixels a sideways wheel notch, or an arrow key, scrolls.
const SIDEWAYS: f32 = 48.0;
/// How near a column's edge, in logical pixels, a press resizes the column.
const EDGE_GRAB: f32 = 4.0;
/// How far, in logical pixels, a pressed title may move and still sort on release.
const CLICK_SLOP: f32 = 4.0;
/// The narrowest a column gets, in characters.
const MIN_CHARS: u32 = 2;

impl<M: GridModel> DataGrid<M> {
    pub(super) fn handle(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        match event {
            Event::Pointer(pointer) => self.pointer(cx, pointer),
            Event::Key(key) if key.pressed => self.key(cx, key),
            Event::FocusGained | Event::FocusLost => {
                cx.request_paint();
                Handled::No
            }
            _ => Handled::No,
        }
    }

    fn pointer(&mut self, cx: &mut EventCx<'_>, pointer: &PointerEvent) -> Handled {
        let at = pointer.position;
        match pointer.kind {
            PointerKind::Scroll(delta) => self.wheel(cx, delta, pointer.modifiers.shift()),
            PointerKind::Down(PointerButton::Primary) => {
                self.press(cx, at, pointer.clicks, pointer.modifiers.shift())
            }
            // A secondary press selects its row first (unless the row is among the selected
            // ones), so the context menu the Ui reports after it is for what is selected.
            PointerKind::Down(PointerButton::Secondary) => match self.row_at(cx, at) {
                Some(row) => {
                    if !self.selected_rows().contains(&row) {
                        self.select_row(cx, row);
                    }
                    Handled::Yes
                }
                None => Handled::No,
            },
            PointerKind::Move => {
                match self.drag {
                    Some(Drag::Column { column, chars, x }) => {
                        self.resize(cx, column, chars, at.x - x);
                    }
                    Some(Drag::Thumb { grab }) => self.drag_thumb(cx, at.y - grab),
                    Some(Drag::Title { .. }) => {}
                    None => {
                        let edge = self.edge_at(cx, at).is_some();
                        cx.set_cursor(edge.then_some(Cursor::ResizeColumn));
                        return Handled::No;
                    }
                }
                Handled::Yes
            }
            // Any release ends the press: the Ui stops sending the grid the pointer.
            PointerKind::Up(button) => self.release(cx, at, button == PointerButton::Primary),
            PointerKind::Cancel => self.release(cx, at, false),
            _ => Handled::No,
        }
    }

    /// The end of a press: a title clicked without moving away sorts.
    fn release(&mut self, cx: &mut EventCx<'_>, at: Point, primary: bool) -> Handled {
        let Some(drag) = self.drag.take() else {
            return Handled::No;
        };
        if let (Drag::Title { column, at: from }, true) = (drag, primary) {
            let slop = cx.px(CLICK_SLOP);
            if (at.x - from.x).abs() <= slop && (at.y - from.y).abs() <= slop {
                self.sort_by(cx, column);
            }
        }
        cx.request_paint();
        Handled::Yes
    }

    fn wheel(&mut self, cx: &mut EventCx<'_>, delta: ScrollDelta, shift: bool) -> Handled {
        let row_height = self.layout_of().map_or(1.0, |l| l.row_height() as f32);
        let sideways = cx.px(SIDEWAYS);
        // Rows down (fractional for touchpads) and pixels right.
        let (rows, dx) = match delta {
            ScrollDelta::Lines(dx, dy) if shift && dx == 0.0 => (0.0, dy * sideways),
            ScrollDelta::Lines(dx, dy) => (dy * WHEEL_ROWS, dx * sideways),
            ScrollDelta::Pixels(dx, dy) if shift && dx == 0.0 => (0.0, dy),
            ScrollDelta::Pixels(dx, dy) => (dy / row_height, dx),
            _ => (0.0, 0.0),
        };
        let mut moved = false;
        if rows != 0.0 && rows.is_finite() {
            // A change of direction drops what was left of the other one.
            if self.row_rest * rows < 0.0 {
                self.row_rest = 0.0;
            }
            self.row_rest += rows;
            let whole = self.row_rest.trunc();
            self.row_rest -= whole;
            let before = self.first_row;
            // Lossless: one event scrolls a few thousand rows at most; the result is clamped.
            let step = whole as isize;
            self.first_row = self
                .first_row
                .saturating_add_signed(step)
                .min(self.last_first_row());
            let at_end = (rows > 0.0 && self.first_row == self.last_first_row())
                || (rows < 0.0 && self.first_row == 0);
            if at_end && self.first_row == before {
                // Nothing left to scroll that way: the views around the grid may.
                self.row_rest = 0.0;
            } else {
                moved = true;
            }
        }
        if dx != 0.0 && dx.is_finite() {
            let max = self.max_scroll_x(cx.rect().width());
            let to = (self.scroll_x + dx).clamp(0.0, max);
            moved |= to != self.scroll_x;
            self.scroll_x = to;
        }
        if !moved {
            return Handled::No;
        }
        cx.request_paint();
        Handled::Yes
    }

    fn press(&mut self, cx: &mut EventCx<'_>, at: Point, clicks: u8, shift: bool) -> Handled {
        let Some(frame) = self.frame(cx.rect()) else {
            return Handled::No;
        };
        cx.request_focus();
        if let Some(thumb) = self.thumb(&frame, cx.scale()) {
            let grab = cx.px(BAR_GRAB).round();
            if at.x >= frame.body.right() - grab && frame.body.contains(at) {
                // On the thumb: drag it from where it was grabbed; beside it: jump there.
                let grab = if thumb.top() <= at.y && at.y < thumb.bottom() {
                    at.y - thumb.top()
                } else {
                    thumb.height() / 2.0
                };
                self.drag = Some(Drag::Thumb { grab });
                self.drag_thumb(cx, at.y - grab);
                return Handled::Yes;
            }
        }
        if let Some(column) = self.edge_at(cx, at) {
            let chars = self.columns[column].chars;
            self.drag = Some(Drag::Column {
                column,
                chars,
                x: at.x,
            });
            return Handled::Yes;
        }
        if frame.header.contains(at) {
            if let Some(column) = self.column_at(cx, at.x) {
                self.drag = Some(Drag::Title { column, at });
            }
            return Handled::Yes;
        }
        match self.row_at(cx, at) {
            Some(row) if shift => self.extend_to(cx, row),
            Some(row) => {
                self.select_row(cx, row);
                if clicks >= 2 {
                    cx.emit(ActionKind::Submitted);
                }
            }
            None => {}
        }
        Handled::Yes
    }

    /// Sorts by `column`: ascending first, then the other way on each click. A selection
    /// with a key stays on its record; one without is cleared (it was a position).
    fn sort_by(&mut self, cx: &mut EventCx<'_>, column: usize) {
        let order = match self.sort_order() {
            Some((c, order)) if c == column => order.flipped(),
            _ => SortOrder::Ascending,
        };
        if !self.model.sort(column, order) {
            return;
        }
        self.sort = Some((column, order));
        self.first_row = 0;
        self.row_rest = 0.0;
        self.anchor = None;
        self.selected = self.selected.filter(|s| s.key.is_some());
        self.clamp();
        if let Some(row) = self.selected() {
            self.reveal_row(row);
        }
        cx.emit(ActionKind::Changed);
        cx.request_paint();
    }

    /// Where `x` (in the window) is among the columns, shown as scrolled: the column whose
    /// cell, gaps halved either side, contains it.
    fn column_at(&self, cx: &EventCx<'_>, x: f32) -> Option<usize> {
        let layout = self.layout_of()?;
        let frame = self.frame(cx.rect())?;
        let local = x - frame.rect.left();
        let half = layout.gap() as f32 / 2.0;
        let count = self.columns.len().min(layout.column_count());
        (0..count).find(|&c| {
            let shift = self.shift_of(c, frame.rect.width());
            let left = layout.column_x(c) as f32 - half - shift;
            let right = left + layout.column_width(c) as f32 + 2.0 * half;
            let visible = c < self.frozen || local >= frame.frozen;
            visible && local >= left && local < right
        })
    }

    /// The column whose right edge, in the header, is under `at`.
    fn edge_at(&self, cx: &EventCx<'_>, at: Point) -> Option<usize> {
        let layout = self.layout_of()?;
        let frame = self.frame(cx.rect())?;
        if !frame.header.contains(at) {
            return None;
        }
        let local = at.x - frame.rect.left();
        let grab = cx.px(EDGE_GRAB);
        let count = self.columns.len().min(layout.column_count());
        (0..count).find(|&c| {
            let shift = self.shift_of(c, frame.rect.width());
            let edge = layout.column_x(c) as f32
                + layout.column_width(c) as f32
                + layout.gap() as f32 / 2.0
                - shift;
            let visible = c < self.frozen || local >= frame.frozen;
            visible && (local - edge).abs() <= grab
        })
    }

    /// How far column `column` is scrolled sideways in a grid `width` wide.
    pub(super) fn shift_of(&self, column: usize, width: f32) -> f32 {
        if column < self.frozen {
            0.0
        } else {
            self.scroll_x.min(self.max_scroll_x(width)).round()
        }
    }

    /// The row under `at`, if it is in the rows there are.
    fn row_at(&self, cx: &EventCx<'_>, at: Point) -> Option<usize> {
        let layout = self.layout_of()?;
        let frame = self.frame(cx.rect())?;
        if !frame.body.contains(at) {
            return None;
        }
        let index = ((at.y - frame.body.top()) / layout.row_height() as f32).floor();
        // Lossless: the index counts rows on screen.
        let row = self.first_row.checked_add(index as usize)?;
        (row < self.model.rows()).then_some(row)
    }

    /// Resizes `column` from `chars` by `dx` pixels, in whole characters.
    fn resize(&mut self, cx: &mut EventCx<'_>, column: usize, chars: u32, dx: f32) {
        let Some(char_width) = self.layout_of().map(|l| l.char_width()) else {
            return;
        };
        // Lossless: column widths are a few hundred characters at most.
        let to = (chars as f32 + (dx / char_width).round()).max(MIN_CHARS as f32) as u32;
        let Some(target) = self.columns.get_mut(column) else {
            return;
        };
        if target.chars == to {
            return;
        }
        target.chars = to;
        // The painter is rebuilt for the new widths at the next layout.
        self.built = None;
        cx.request_layout();
        cx.request_paint();
    }

    /// Scrolls so the thumb's top is at `top` (in the window).
    fn drag_thumb(&mut self, cx: &mut EventCx<'_>, top: f32) {
        let Some(frame) = self.frame(cx.rect()) else {
            return;
        };
        let Some(thumb) = self.thumb(&frame, cx.scale()) else {
            return;
        };
        let travel = (frame.body.height() - thumb.height()).max(1.0);
        let at = f64::from(((top - frame.body.top()) / travel).clamp(0.0, 1.0));
        // In f64: row counts past 2^24 lose rows in f32. Lossless: at most the row count.
        let first = (at * self.last_first_row() as f64).round() as usize;
        if first != self.first_row {
            self.first_row = first;
            self.row_rest = 0.0;
            cx.request_paint();
        }
    }

    /// The selection keys (with Shift: extending it), Ctrl+A and Ctrl+C (Cmd on macOS),
    /// Left and Right to scroll sideways, and Enter to activate the selected row, while the
    /// grid has focus. Other keys with modifiers are left to the application.
    fn key(&mut self, cx: &mut EventCx<'_>, key: &KeyEvent<'_>) -> Handled {
        let m = key.modifiers;
        if let Some(handled) = self.command(cx, key) {
            return handled;
        }
        if m.control() || m.alt() || m.logo() {
            return Handled::No;
        }
        let extend = m.shift();
        let rows = self.model.rows();
        let sideways = cx.px(SIDEWAYS);
        let max_x = self.max_scroll_x(cx.rect().width());
        match key.key {
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight) if !extend => {
                let dx = if key.key == Key::Named(NamedKey::ArrowLeft) {
                    -sideways
                } else {
                    sideways
                };
                let to = (self.scroll_x + dx).clamp(0.0, max_x);
                if to == self.scroll_x {
                    return Handled::No;
                }
                self.scroll_x = to;
                cx.request_paint();
                return Handled::Yes;
            }
            Key::Named(NamedKey::Enter) if !extend => {
                if self.selected().is_none() {
                    return Handled::No;
                }
                cx.emit(ActionKind::Submitted);
                return Handled::Yes;
            }
            _ => {}
        }
        if rows == 0 {
            return Handled::No;
        }
        let page = self.whole_rows().max(1);
        // Without a selection, moving starts from the first row on screen.
        let at = self.selected();
        let from = at.unwrap_or(self.first_row);
        let to = match key.key {
            Key::Named(NamedKey::ArrowDown) => at.map_or(from, |r| r.saturating_add(1)),
            Key::Named(NamedKey::ArrowUp) => at.map_or(from, |r| r.saturating_sub(1)),
            Key::Named(NamedKey::PageDown) => from.saturating_add(page),
            Key::Named(NamedKey::PageUp) => from.saturating_sub(page),
            Key::Named(NamedKey::Home) => 0,
            Key::Named(NamedKey::End) => rows - 1,
            _ => return Handled::No,
        }
        .min(rows - 1);
        if extend {
            self.extend_to(cx, to);
        } else {
            self.select_row(cx, to);
        }
        Handled::Yes
    }

    /// Select all and copy: `None` for other keys.
    fn command(&mut self, cx: &mut EventCx<'_>, key: &KeyEvent<'_>) -> Option<Handled> {
        let m = key.modifiers;
        if m.alt() || m.shift() {
            return None;
        }
        let copy = match key.key {
            Key::Character(c) if m.command() && c.eq_ignore_ascii_case("a") => {
                if self.model.rows() == 0 {
                    return Some(Handled::No);
                }
                self.select_all(cx);
                return Some(Handled::Yes);
            }
            Key::Character(c) => m.command() && c.eq_ignore_ascii_case("c"),
            Key::Named(NamedKey::Insert) => m.control(),
            _ => false,
        };
        if !copy {
            return None;
        }
        Some(if self.copy(cx) {
            Handled::Yes
        } else {
            Handled::No
        })
    }
}
