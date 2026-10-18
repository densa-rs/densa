//! A data grid widget: rows from a [`GridModel`], scrolled, selected and sorted by the user.

mod access;
mod input;
mod paint;

use densa_core::{Color, Point, Rect, Size};
use densa_draw::Canvas;
use densa_grid::{
    CellStyle, CellText, Column, GridLayout, GridModel, GridPainter, GridSource, Palette, SortOrder,
};
use densa_text::Font;
use densa_ui::{
    AccessNode, AccessRequest, Event, EventCx, Handled, LayoutCx, Length, Overflow, PaintCx, Style,
    Theme, Widget,
};

use crate::paint::luminance;

/// What a press on the grid is doing while the button is held.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    /// A press on `column`'s title at `at`: it sorts on release, unless it moved away.
    Title { column: usize, at: Point },
    /// Resizing `column` from `chars` characters, grabbed at `x`.
    Column { column: usize, chars: u32, x: f32 },
    /// Dragging the scroll bar's thumb, grabbed `grab` below its top.
    Thumb { grab: f32 },
}

/// The selected row, and the model's key for its record (if the model has keys), which keeps
/// the selection on the record as rows move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    row: usize,
    key: Option<u64>,
}

/// A grid of rows from a [`GridModel`]: the user scrolls it (wheel, touchpad, scroll bar,
/// keys), selects a row (click, arrow keys, Page Up and Down, Home, End), sorts it by
/// clicking a column's title (again to reverse), and resizes columns by dragging the edges
/// between titles. Leading columns can stay in place while the rest scroll sideways
/// ([`with_frozen_columns`](Self::with_frozen_columns)).
///
/// Only the rows on screen are asked of the model, each frame, and painting allocates nothing
/// once their characters are cached, however many rows there are. Change the data through
/// [`model_mut`](Self::model_mut) from an [`Ui::update`](densa_ui::Ui::update) on the grid,
/// which repaints it. The grid follows the model's [`GridModel::columns_revision`] and
/// [`GridModel::generation`] each frame.
///
/// A selection is a row, kept on its record while rows move if the model gives rows keys
/// ([`GridModel::row_key`]) and cleared when its record is removed or filtered out (as in
/// Qt's proxy models); without keys, sorting clears it. It reports
/// [`ActionKind::Selected`](densa_ui::ActionKind::Selected) with the row when the user
/// selects one, [`ActionKind::Submitted`](densa_ui::ActionKind::Submitted) when they activate
/// it (Enter, a double click), and [`ActionKind::Changed`](densa_ui::ActionKind::Changed) when
/// a click sorted the rows. Give it [`DataGrid::style`].
#[derive(Debug)]
pub struct DataGrid<M> {
    model: M,
    /// The model's columns, as the user resized them.
    columns: Vec<Column>,
    frozen: usize,
    /// Built at layout for the rows that fit, at the theme's text size.
    painter: Option<GridPainter>,
    first_row: usize,
    /// Rows scrolled by a touchpad but not yet whole, as a fraction of a row.
    row_rest: f32,
    /// How far the unfrozen columns are scrolled sideways, in physical pixels.
    scroll_x: f32,
    selected: Option<Selection>,
    /// The sort a click made, for models that do not report theirs.
    sort: Option<(usize, SortOrder)>,
    drag: Option<Drag>,
    /// What the last layout used, to rebuild the painter when it changes.
    built: Option<(Size, f32, Palette)>,
    /// The model's [`GridModel::columns_revision`] and [`GridModel::generation`] last seen.
    revisions: (u64, u64),
}

impl<M: GridModel> DataGrid<M> {
    /// A grid showing `model`.
    pub fn new(model: M) -> Self {
        let columns = model.columns().to_vec();
        Self {
            revisions: (model.columns_revision(), model.generation()),
            model,
            columns,
            frozen: 0,
            painter: None,
            first_row: 0,
            row_rest: 0.0,
            scroll_x: 0.0,
            selected: None,
            sort: None,
            drag: None,
            built: None,
        }
    }

    /// The grid with its first `columns` columns kept in place while the others scroll
    /// sideways (all of them, if there are fewer).
    #[must_use]
    pub fn with_frozen_columns(mut self, columns: usize) -> Self {
        self.frozen = columns;
        self
    }

    /// A grid filling its parent's free space, allowed to be smaller than its rows, and
    /// clipping them.
    pub fn style() -> Style {
        Style::column()
            .grow(1.0)
            .min_size(Length::Px(0.0), Length::Px(0.0))
            .overflow(Overflow::Clip)
    }

    /// The data.
    pub fn model(&self) -> &M {
        &self.model
    }

    /// The data, to change (from an [`Ui::update`](densa_ui::Ui::update) on the grid): the
    /// grid repaints, and keeps its selection and scroll within the rows there are.
    pub fn model_mut(&mut self, cx: &mut EventCx<'_>) -> &mut M {
        cx.request_paint();
        &mut self.model
    }

    /// Reads the model's columns again (after it changed them), dropping the user's widths,
    /// and a sort or a drag on a column that is gone.
    pub fn reset_columns(&mut self, cx: &mut EventCx<'_>) {
        self.read_columns();
        self.built = None;
        cx.request_layout();
        cx.request_paint();
    }

    /// The selected row.
    pub fn selected(&self) -> Option<usize> {
        self.selected.map(|s| s.row)
    }

    /// Selects `row` (none for `None`) and scrolls it into view.
    pub fn select(&mut self, cx: &mut EventCx<'_>, row: Option<usize>) {
        self.selected = row
            .filter(|r| *r < self.model.rows())
            .map(|row| self.selection_of(row));
        if let Some(row) = row {
            self.reveal_row(row);
        }
        cx.request_paint();
    }

    /// The first row on screen.
    pub fn first_row(&self) -> usize {
        self.first_row
    }

    /// Scrolls so that `row` is the first on screen, or as near as the rows allow.
    pub fn scroll_to_row(&mut self, cx: &mut EventCx<'_>, row: usize) {
        self.first_row = row.min(self.last_first_row());
        self.row_rest = 0.0;
        cx.request_paint();
    }

    /// The column and order the rows are sorted by: the model's own, if it reports it, else
    /// the last one a click asked for.
    pub fn sort_order(&self) -> Option<(usize, SortOrder)> {
        self.model.sort_state().or(self.sort)
    }

    /// The columns as shown, with the user's widths.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// How many rows are painted, the last one possibly cut off (0 before the first layout).
    pub fn visible_rows(&self) -> usize {
        self.painter
            .as_ref()
            .map_or(0, |p| p.layout().rows() as usize)
    }

    /// How many rows fit whole: what a page is, and how many the last screen shows.
    pub fn whole_rows(&self) -> usize {
        let (Some(layout), Some((size, _, _))) = (self.layout_of(), self.built) else {
            return 0;
        };
        let header = (layout.padding() + layout.row_height()) as f32;
        let rows = ((size.height - header) / layout.row_height() as f32).floor();
        // Lossless: a screen holds a few hundred rows at most; negative and NaN become 0.
        rows.max(0.0) as usize
    }

    fn layout_of(&self) -> Option<&GridLayout> {
        self.painter.as_ref().map(GridPainter::layout)
    }

    /// The highest first row: the last rows fill the screen.
    fn last_first_row(&self) -> usize {
        self.model.rows().saturating_sub(self.whole_rows().max(1))
    }

    /// Scrolls the least that shows `row` whole.
    fn reveal_row(&mut self, row: usize) {
        let whole = self.whole_rows().max(1);
        if row < self.first_row {
            self.first_row = row;
        } else if row >= self.first_row + whole {
            self.first_row = row + 1 - whole;
        }
        self.row_rest = 0.0;
    }

    /// The selection of `row`, with its record's key.
    fn selection_of(&self, row: usize) -> Selection {
        Selection {
            row,
            key: self.model.row_key(row),
        }
    }

    /// The model's columns again, ending a drag and a sort on a column that is gone.
    fn read_columns(&mut self) {
        self.columns = self.model.columns().to_vec();
        let count = self.columns.len();
        self.drag = None;
        self.sort = self.sort.filter(|(column, _)| *column < count);
    }

    /// Takes in what the model replaced since the last frame: its columns (the painter is
    /// rebuilt), or all its rows (the selection and scroll start over).
    fn follow_model(&mut self) {
        let (columns, generation) = (self.model.columns_revision(), self.model.generation());
        if columns != self.revisions.0 {
            self.read_columns();
            if let Some((size, px, palette)) = self.built {
                self.build(size, px, palette);
            }
        }
        if generation != self.revisions.1 {
            self.selected = None;
            self.sort = None;
            self.drag = None;
            self.first_row = 0;
            self.row_rest = 0.0;
            self.scroll_x = 0.0;
        }
        self.revisions = (columns, generation);
    }

    /// Keeps the selection on its record, and the scroll within the rows and columns there
    /// are.
    fn clamp(&mut self) {
        let rows = self.model.rows();
        self.selected = self.selected.and_then(|s| match s.key {
            Some(key) => self.model.row_of_key(key).map(|row| Selection { row, ..s }),
            None => Some(s),
        });
        self.selected = self.selected.filter(|s| s.row < rows);
        self.first_row = self.first_row.min(self.last_first_row());
        if let Some((size, _, _)) = self.built {
            self.scroll_x = self.scroll_x.clamp(0.0, self.max_scroll_x(size.width));
        }
    }

    /// The painter for `size` at `px`, or none if the bundled font cannot be read.
    fn build(&mut self, size: Size, px: f32, palette: Palette) {
        let Ok(font) = Font::bundled_mono() else {
            self.painter = None;
            return;
        };
        // One row first, to learn the row height the font gives.
        let probe = GridPainter::new(self.columns.clone(), 1, font.clone(), px, palette);
        let layout = probe.layout();
        let header = (layout.padding() + layout.row_height()) as f32;
        let rows = ((size.height - header) / layout.row_height() as f32).ceil();
        // Lossless: a screen holds a few hundred rows at most; negative and NaN become 0.
        let rows = rows.max(0.0) as u32;
        self.painter = Some(GridPainter::new(
            self.columns.clone(),
            rows,
            font,
            px,
            palette,
        ));
        self.built = Some((size, px, palette));
    }
}

/// The grid's colors from the theme.
fn palette(theme: &Theme) -> Palette {
    let mut palette = Palette::new();
    palette.background = theme.surface;
    palette.header_background = theme.background;
    palette.header_text = theme.text_dim;
    palette.stripe = theme.text.with_alpha(0.035);
    palette.text = theme.text;
    palette.dim_text = theme.text_dim;
    // Darker green and red on a light surface, where the dark theme's are too faint.
    if luminance(theme.surface) > 0.5 {
        palette.positive = Color::srgb(0x1A7F37);
        palette.negative = Color::srgb(0xCF222E);
    }
    palette
}

/// The rows on screen, as the painter reads them: from the first row, the selected one
/// highlighted where the model draws no fill of its own (a flash stays visible).
struct Shown<'a, M> {
    model: &'a M,
    first: usize,
    selected: Option<usize>,
    highlight: Color,
    palette: Palette,
}

impl<M: GridModel> GridSource for Shown<'_, M> {
    fn rows(&self) -> usize {
        self.model.rows().saturating_sub(self.first)
    }

    fn cell(&self, row: usize, column: usize, text: &mut CellText) -> CellStyle {
        let row = self.first + row;
        let style = self.model.cell(row, column, text, &self.palette);
        if self.selected == Some(row) && style.background.is_none() {
            return style.with_background(self.highlight);
        }
        style
    }
}

impl<M: GridModel + 'static> Widget for DataGrid<M> {
    fn layout(&mut self, cx: &mut LayoutCx<'_>, size: Size) {
        let px = cx.px(cx.theme().font_size);
        let palette = palette(cx.theme());
        if self.built != Some((size, px, palette)) {
            self.build(size, px, palette);
        }
        self.follow_model();
        self.clamp();
    }

    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        self.model.prepare();
        self.follow_model();
        self.clamp();
        self.paint_grid(cx, canvas);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        self.handle(cx, event)
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn accessibility(&self, cx: &PaintCx<'_>, node: &mut AccessNode) {
        self.describe(cx, node);
    }

    fn access_action(&mut self, cx: &mut EventCx<'_>, request: &AccessRequest<'_>) -> Handled {
        self.act(cx, request)
    }
}

/// Where the grid's parts are within `rect`, in physical pixels.
#[derive(Debug, Clone, Copy)]
struct Frame {
    /// The whole widget.
    rect: Rect,
    /// The header row, across the widget.
    header: Rect,
    /// The rows below the header, across the widget.
    body: Rect,
    /// Where the frozen columns end, from the widget's left (0 when none are frozen).
    frozen: f32,
}
