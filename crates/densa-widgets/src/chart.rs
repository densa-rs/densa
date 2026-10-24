//! A chart widget: the panes of a [`ChartModel`] on one horizontal axis, following the
//! newest data until the user pans or zooms.

use densa_chart::{Axis, ChartModel, ChartPainter, ChartPalette, Plot};
use densa_core::{Color, Point, Rect, Size};
use densa_draw::Canvas;
use densa_text::Font;
use densa_ui::{
    AccessNode, ActionKind, Event, EventCx, Handled, Key, KeyEvent, LayoutCx, Length, NamedKey,
    Overflow, PaintCx, PointerButton, PointerEvent, PointerKind, Role, ScrollDelta, Style, Theme,
    Widget,
};

use crate::paint::{luminance, outline};

/// How much one wheel notch zooms.
const ZOOM_STEP: f64 = 1.15;
/// Logical pixels of touchpad scrolling that count as one wheel notch.
const NOTCH: f32 = 48.0;
/// The share of the view an arrow key pans.
const KEY_PAN: f64 = 0.1;
/// Room right of the newest data while following, as a share of the view.
const LEAD: f64 = 0.03;
/// Points a view shows at least, when the model gives their distance.
const MIN_STEPS: f64 = 5.0;
/// The narrowest view, as a share of the data, when the model gives no distance.
const MIN_SHARE: f64 = 1e-4;
/// How far, in logical pixels, a press may move and still count toward a double click.
const CLICK_SLOP: f32 = 4.0;
/// The narrowest view, as a share of the magnitude of `x`: an `f64` tells apart about a
/// ten-thousandth of this, so views stay far wider than the values' precision.
const PRECISION: f64 = 1e-12;
/// Characters the `y` labels get room for, unless told otherwise.
const LABEL_CHARS: u16 = 8;
/// The widest view, in widths of the data.
const MAX_SHARE: f64 = 2.0;

/// What part of the horizontal axis shows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum View {
    /// The newest `span` of the data, moving with it.
    Follow { span: f64 },
    /// A range the user panned or zoomed to, kept as data arrives.
    Fixed(Axis),
}

/// A press dragging the view: grabbed at `x` while `from` showed across `width` pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pan {
    x: f32,
    from: Axis,
    width: f32,
}

/// A chart of a [`ChartModel`]: its panes stacked top to bottom on one horizontal axis,
/// labelled under the lowest, each scaled to what it shows.
///
/// It shows the newest `span` of the data and moves with it as data arrives. The user zooms
/// with the wheel or the up and down keys (around the pointer, or at the newest data while
/// following it), pans by dragging, with a sideways scroll or with the left and right keys,
/// and goes back to the newest data with a double click, End, or by panning past it; Home
/// goes to the oldest. A view the user moved stays over the data as it changes (data
/// trimmed away moves it along). A crosshair follows the pointer with its values, written a
/// digit finer than the labels ([`with_label_chars`](Self::with_label_chars) for room). It
/// reports [`ActionKind::Changed`] when the user moved the view.
///
/// Painting allocates nothing once the label characters are cached; what the model's
/// series cost is up to them (this crate's series functions allocate nothing). Change the
/// data through [`model_mut`](Self::model_mut) from an [`Ui::update`](densa_ui::Ui::update)
/// on the chart, which repaints it. Give it [`Chart::style`].
#[derive(Debug)]
pub struct Chart<M> {
    model: M,
    view: View,
    /// The span shown at first: the widest view while the data is narrower.
    initial: f64,
    label_chars: u16,
    /// Built at layout, at the theme's text size and colors.
    painter: Option<ChartPainter>,
    built: Option<(f32, ChartPalette)>,
    pan: Option<Pan>,
    /// Whether the last press dragged the pointer: the press after it starts another drag,
    /// not a double click.
    dragged: bool,
    /// Where the pointer is over the chart, for the crosshair.
    hover: Option<Point>,
}

impl<M: ChartModel> Chart<M> {
    /// A chart of `model` showing its newest `span` of `x` (an hour is `3600.0` on a time
    /// axis), following new data.
    pub fn new(model: M, span: f64) -> Self {
        let span = if span.is_finite() && span > 0.0 {
            span
        } else {
            1.0
        };
        Self {
            model,
            view: View::Follow { span },
            initial: span,
            label_chars: LABEL_CHARS,
            painter: None,
            built: None,
            pan: None,
            dragged: false,
            hover: None,
        }
    }

    /// The chart with room for `y` labels of `chars` characters (8 unless told otherwise).
    #[must_use]
    pub fn with_label_chars(mut self, chars: u16) -> Self {
        self.label_chars = chars;
        self.built = None;
        self
    }

    /// A chart filling its parent's free space, allowed to be small, and clipping.
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

    /// The data, to change (from an [`Ui::update`](densa_ui::Ui::update) on the chart): the
    /// chart repaints.
    pub fn model_mut(&mut self, cx: &mut EventCx<'_>) -> &mut M {
        cx.request_paint();
        &mut self.model
    }

    /// What part of the horizontal axis shows, or `None` while there is no data.
    pub fn x_axis(&self) -> Option<Axis> {
        let (_, newest) = self.extent()?;
        let kind = self.model.x_kind();
        Some(match self.current() {
            View::Follow { span } => {
                let end = newest + span * LEAD;
                Axis::new(end - span, end, kind)
            }
            View::Fixed(axis) => Axis::new(axis.start(), axis.end(), kind),
        })
    }

    /// Whether the chart moves with new data.
    pub fn is_following(&self) -> bool {
        matches!(self.current(), View::Follow { .. })
    }

    /// The view, kept over the data as it is now.
    fn current(&self) -> View {
        match self.view {
            View::Fixed(axis) => self.settle(axis.start(), axis.span()),
            follow @ View::Follow { .. } => follow,
        }
    }

    /// Shows the newest data again, as wide a part as now, and follows it.
    pub fn follow(&mut self, cx: &mut EventCx<'_>) {
        if let Some(x) = self.x_axis() {
            self.view = View::Follow { span: x.span() };
            cx.request_paint();
        }
    }

    /// Shows `x` (within what the data allows), following new data if it reaches past the
    /// newest.
    pub fn show(&mut self, cx: &mut EventCx<'_>, x: Axis) {
        let (low, high) = self.limits();
        let span = x.span().clamp(low, high);
        let middle = x.start() + x.span() / 2.0;
        self.view = self.settle(middle - span / 2.0, span);
        cx.request_paint();
    }

    /// The data's lowest and highest `x`, if it has any (finite, in order).
    fn extent(&self) -> Option<(f64, f64)> {
        self.model
            .x_extent()
            .filter(|(low, high)| low.is_finite() && high.is_finite() && low <= high)
    }

    /// The narrowest and widest views the data allows.
    fn limits(&self) -> (f64, f64) {
        let Some((oldest, newest)) = self.extent() else {
            return (f64::MIN_POSITIVE, f64::MAX);
        };
        let width = newest - oldest;
        let low = match self.model.x_step() {
            Some(step) if step.is_finite() && step > 0.0 => step * MIN_STEPS,
            _ => width * MIN_SHARE,
        };
        let magnitude = oldest.abs().max(newest.abs()).max(1.0);
        let low = low.max(magnitude * PRECISION);
        let high = (width * MAX_SHARE).max(self.initial).max(low * 2.0);
        (low, high)
    }

    /// The view `span` wide from `start`, kept over the data: it ends no earlier than the
    /// oldest, and follows once it reaches past the newest.
    fn settle(&self, start: f64, span: f64) -> View {
        let kind = self.model.x_kind();
        let Some((oldest, newest)) = self.extent() else {
            return View::Fixed(Axis::new(start, start + span, kind));
        };
        let start = start.max(oldest - span);
        if start + span >= newest + span * LEAD {
            return View::Follow { span };
        }
        View::Fixed(Axis::new(start, start + span, kind))
    }

    /// Zooms by `factor` (below 1 in) around canvas `x`, or around the newest data while
    /// following. Whether the view changed.
    fn zoom(&mut self, factor: f64, x: Option<f32>, rect: Rect) -> bool {
        let Some(shown) = self.x_axis() else {
            return false;
        };
        let (low, high) = self.limits();
        let span = (shown.span() * factor).clamp(low, high);
        if span == shown.span() || !span.is_finite() {
            return false;
        }
        let area = self.x_area(rect);
        let anchor = match (self.current(), x, area) {
            (View::Fixed(_), Some(x), Some(area)) if (area.x0..area.x1).contains(&x) => {
                Plot::new(area, shown, shown).x_value(x)
            }
            (View::Fixed(_), _, _) => shown.start() + shown.span() / 2.0,
            (View::Follow { .. }, _, _) => {
                self.view = View::Follow { span };
                return true;
            }
        };
        let zoomed = shown.zoomed(span / shown.span(), anchor);
        self.view = self.settle(zoomed.start(), span);
        true
    }

    /// Pans by `dx` canvas pixels (positive shows later data). Whether the view changed.
    fn pan_by(&mut self, dx: f32, rect: Rect) -> bool {
        let (Some(shown), Some(area)) = (self.x_axis(), self.x_area(rect)) else {
            return false;
        };
        let delta = f64::from(dx) / f64::from(area.width()) * shown.span();
        self.shift(shown, delta)
    }

    /// Shows `shown` moved by `delta`. Whether the view changed.
    fn shift(&mut self, shown: Axis, delta: f64) -> bool {
        if delta == 0.0 || !delta.is_finite() {
            return false;
        }
        let before = self.view;
        self.view = self.settle(shown.start() + delta, shown.span());
        self.view != before
    }

    /// Where the plots are across `rect` (all panes share their left and right edges).
    fn x_area(&mut self, rect: Rect) -> Option<Rect> {
        let area = self.painter.as_mut()?.plot_area(rect, true);
        (area.width() > 0.0).then_some(area)
    }

    /// The painter for text of `px` pixels in `palette`, or none if the bundled font cannot
    /// be read.
    fn build(&mut self, px: f32, palette: ChartPalette) {
        let chars = self.label_chars;
        self.painter = Font::bundled_mono()
            .ok()
            .map(|font| ChartPainter::new(font, px, palette).with_label_chars(chars));
        self.built = Some((px, palette));
    }

    fn pointer(&mut self, cx: &mut EventCx<'_>, pointer: &PointerEvent) -> Handled {
        let at = pointer.position;
        let rect = cx.rect();
        let moved = match pointer.kind {
            PointerKind::Scroll(delta) => {
                self.wheel(cx, delta, at, pointer.modifiers.shift(), rect)
            }
            PointerKind::Down(PointerButton::Primary) if pointer.clicks >= 2 && !self.dragged => {
                self.pan = None;
                let following = self.is_following();
                self.follow(cx);
                !following
            }
            PointerKind::Down(PointerButton::Primary) => {
                self.dragged = false;
                if let (Some(from), Some(area)) = (self.x_axis(), self.x_area(rect)) {
                    let width = area.width();
                    self.pan = Some(Pan {
                        x: at.x,
                        from,
                        width,
                    });
                }
                cx.request_paint();
                return Handled::Yes;
            }
            PointerKind::Move => match self.pan {
                Some(pan) => {
                    self.hover = Some(at);
                    self.dragged |= (at.x - pan.x).abs() > cx.px(CLICK_SLOP);
                    let delta = f64::from(pan.x - at.x) / f64::from(pan.width) * pan.from.span();
                    let before = self.view;
                    self.view = self.settle(pan.from.start() + delta, pan.from.span());
                    self.view != before
                }
                None => {
                    self.hover = Some(at);
                    cx.request_paint();
                    return Handled::No;
                }
            },
            PointerKind::Leave => {
                self.hover = None;
                cx.request_paint();
                return Handled::No;
            }
            PointerKind::Up(_) | PointerKind::Cancel => {
                let held = self.pan.take().is_some();
                cx.request_paint();
                return if held { Handled::Yes } else { Handled::No };
            }
            _ => return Handled::No,
        };
        self.moved(cx, moved)
    }

    /// A wheel zooms around the pointer; a sideways scroll, or one with Shift, pans.
    fn wheel(
        &mut self,
        cx: &mut EventCx<'_>,
        delta: ScrollDelta,
        at: Point,
        shift: bool,
        rect: Rect,
    ) -> bool {
        let notch = cx.px(NOTCH);
        let (zoom, dx) = match delta {
            ScrollDelta::Lines(dx, dy) if shift && dx == 0.0 => (0.0, dy * notch),
            ScrollDelta::Lines(dx, dy) => (dy, dx * notch),
            ScrollDelta::Pixels(dx, dy) if shift && dx == 0.0 => (0.0, dy),
            ScrollDelta::Pixels(dx, dy) => (dy / notch, dx),
            _ => (0.0, 0.0),
        };
        let mut moved = false;
        if zoom != 0.0 && zoom.is_finite() {
            moved |= self.zoom(ZOOM_STEP.powf(f64::from(zoom)), Some(at.x), rect);
        }
        if dx != 0.0 && dx.is_finite() {
            moved |= self.pan_by(dx, rect);
        }
        moved
    }

    fn key(&mut self, cx: &mut EventCx<'_>, key: &KeyEvent<'_>) -> Handled {
        let m = key.modifiers;
        if m.shift() || m.control() || m.alt() || m.logo() {
            return Handled::No;
        }
        let rect = cx.rect();
        let Some(shown) = self.x_axis() else {
            return Handled::No;
        };
        let moved = match key.key {
            Key::Named(NamedKey::ArrowLeft) => self.shift(shown, -shown.span() * KEY_PAN),
            Key::Named(NamedKey::ArrowRight) => self.shift(shown, shown.span() * KEY_PAN),
            Key::Named(NamedKey::ArrowUp) => self.zoom(1.0 / ZOOM_STEP, None, rect),
            Key::Named(NamedKey::ArrowDown) => self.zoom(ZOOM_STEP, None, rect),
            Key::Named(NamedKey::End) => {
                let following = self.is_following();
                self.follow(cx);
                !following
            }
            Key::Named(NamedKey::Home) => {
                let oldest = self.extent().map_or(shown.start(), |(oldest, _)| oldest);
                let before = self.view;
                self.view = self.settle(oldest, shown.span());
                self.view != before
            }
            _ => return Handled::No,
        };
        self.moved(cx, moved);
        Handled::Yes
    }

    /// Repaints and reports a view the user moved.
    fn moved(&mut self, cx: &mut EventCx<'_>, moved: bool) -> Handled {
        if moved {
            cx.request_paint();
            cx.emit(ActionKind::Changed);
            Handled::Yes
        } else {
            Handled::No
        }
    }

    /// Draws every pane, the crosshair over them, and the focus ring.
    fn paint_chart(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let rect = cx.rect();
        // Data may have moved under a view the user chose: keep it over the data.
        self.view = self.current();
        let x = self.x_axis();
        let hover = self.hover.filter(|_| self.pan.is_none());
        let inside = canvas.state();
        canvas.clip(rect);
        if let Some((_, palette)) = self.built {
            canvas.fill_rect(rect, palette.background);
        }
        let Some(painter) = self.painter.as_mut() else {
            canvas.restore(inside);
            return;
        };
        if let Some(x) = x {
            let panes = self.model.panes().max(1);
            let weight = |pane| {
                let weight = f64::from(self.model.pane_weight(pane));
                if weight.is_finite() {
                    weight.max(0.0)
                } else {
                    0.0
                }
            };
            let total: f64 = (0..panes).map(weight).sum();
            let (mut top, mut before) = (f64::from(rect.top()), 0.0);
            for pane in 0..panes {
                before += if total > 0.0 {
                    weight(pane) / total
                } else {
                    // Lossless enough: a handful of panes.
                    1.0 / panes as f64
                };
                let last = pane + 1 == panes;
                // Lossless enough: pixels.
                let bottom = f64::from(rect.top()) + f64::from(rect.height()) * before;
                let bounds = Rect::from_edges(rect.left(), top as f32, rect.right(), bottom as f32);
                top = bottom;
                let title = self.model.pane_title(pane);
                let bounds = if title.is_empty() {
                    bounds
                } else {
                    // A pane shorter than the line keeps an empty plot, not an inverted one.
                    let below = (bounds.y0 + painter.title_height()).min(bounds.y1);
                    Rect::from_edges(bounds.x0, below, bounds.x1, bounds.y1)
                };
                let area = painter.plot_area(bounds, last);
                let y = self
                    .model
                    .y_axis(pane, &x)
                    .unwrap_or_else(|| Axis::number(0.0, 1.0));
                let plot = Plot::new(area, x, y);
                let model = &self.model;
                let names = |index| model.category(index);
                painter.paint_axes_named(canvas, &plot, last, &names);
                model.paint(pane, &plot, canvas, painter.palette());
                painter.paint_title(canvas, &plot, title);
                if let Some(at) = hover {
                    painter.paint_crosshair_named(canvas, &plot, at, last, &names);
                }
            }
        }
        if cx.is_focused() {
            let line = cx.px(1.0).round().max(1.0);
            outline(canvas, rect, line, cx.theme().focus);
        }
        canvas.restore(inside);
    }
}

/// The chart's colors from the theme.
fn palette(theme: &Theme) -> ChartPalette {
    let mut palette = ChartPalette::new();
    palette.background = theme.surface;
    palette.grid = theme.text.with_alpha(0.06);
    palette.axis = theme.border;
    palette.label = theme.text_dim;
    palette.line = theme.accent;
    palette.crosshair = theme.text_dim.with_alpha(0.7);
    palette.marker = theme.raised;
    palette.marker_text = theme.text;
    // Darker green and red on a light surface, as the grid has.
    if luminance(theme.surface) > 0.5 {
        palette.up = Color::srgb(0x1A7F37);
        palette.down = Color::srgb(0xCF222E);
    }
    palette
}

impl<M: ChartModel + 'static> Widget for Chart<M> {
    fn layout(&mut self, cx: &mut LayoutCx<'_>, _size: Size) {
        let px = cx.px(cx.theme().font_size * 0.9).round();
        let palette = palette(cx.theme());
        if self.built != Some((px, palette)) {
            self.build(px, palette);
        }
    }

    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        self.model.prepare();
        self.paint_chart(cx, canvas);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
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

    fn accepts_focus(&self) -> bool {
        true
    }

    fn accessibility(&self, _cx: &PaintCx<'_>, node: &mut AccessNode) {
        node.set_role(Role::Image);
        let description = self.model.describe();
        node.set_label(if description.is_empty() {
            "Chart".to_owned()
        } else {
            description
        });
    }
}
