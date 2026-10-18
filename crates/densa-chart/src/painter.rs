use densa_core::{Color, Point, Rect};
use densa_draw::Canvas;
use densa_fmt::FmtBuf;
use densa_text::{Font, GlyphCache, GlyphMap, SUBPIXEL_STEPS};

use crate::{AxisKind, ChartPalette, Plot, Ticks};

/// Bytes a tick label may take.
const LABEL_BYTES: usize = 32;

/// Every character a tick label can use.
const LABEL_CHARS: &str = "0123456789-.:";

/// Characters an x label is given room for (a date, and a gap to the next one).
const X_LABEL_ROOM: f32 = 11.0;

/// Characters an x label below a second is given room for (`HH:MM:SS.fff` and a gap).
const X_FINE_LABEL_ROOM: f32 = 15.0;

/// Line heights between two y ticks, at least.
const Y_TICK_LINES: f32 = 2.5;

/// How many times more ticks a crosshair's value is written for: one digit finer.
const FINER: usize = 10;

/// Draws what is around a chart's series: gridlines at the ticks, the axis edges and the
/// tick labels, right of the plot for `y` and under it for `x`, and a crosshair over it.
///
/// Owns the label font's character map; glyphs come from the canvas's shared cache.
/// Painting allocates nothing once the label characters are cached (see
/// [`prewarm`](Self::prewarm)).
#[derive(Debug)]
pub struct ChartPainter {
    glyphs: GlyphMap,
    palette: ChartPalette,
    label: FmtBuf<LABEL_BYTES>,
    label_chars: f32,
}

impl ChartPainter {
    /// A painter writing labels in `font` at `px` pixels per em, with room for `y` labels
    /// of 8 characters.
    pub fn new(font: Font, px: f32, palette: ChartPalette) -> Self {
        Self {
            glyphs: GlyphMap::new(font, px),
            palette,
            label: FmtBuf::new(),
            label_chars: 8.0,
        }
    }

    /// The painter with room for `y` labels of `chars` characters.
    #[must_use]
    pub fn with_label_chars(mut self, chars: u16) -> Self {
        self.label_chars = f32::from(chars.max(1));
        self
    }

    /// The colors this painter draws with.
    pub fn palette(&self) -> &ChartPalette {
        &self.palette
    }

    /// Draws with `palette` from now on.
    pub fn set_palette(&mut self, palette: ChartPalette) {
        self.palette = palette;
    }

    /// Rasterizes every character a label can use into `cache` at every subpixel position
    /// now, so frames never rasterize (or allocate) for labels. Returns how many glyphs could
    /// not be cached.
    pub fn prewarm(&mut self, cache: &mut GlyphCache) -> usize {
        let mut missing = 0;
        for ch in LABEL_CHARS.chars() {
            let Some(mapped) = self.glyphs.get(ch) else {
                continue;
            };
            for bin in 0..SUBPIXEL_STEPS {
                let cached = cache.get(self.glyphs.font(), mapped.id, self.glyphs.px(), bin);
                missing += usize::from(cached.is_err());
            }
        }
        missing
    }

    /// The area of `bounds` left for the plot once the `y` labels (right) and, if
    /// `x_labels`, the `x` labels (below) have their room; edges on whole pixels. Charts
    /// stacked on one time axis label `x` under the lowest only.
    pub fn plot_area(&mut self, bounds: Rect, x_labels: bool) -> Rect {
        let right = (self.char_width() * self.label_chars + 2.0 * self.pad()).round();
        let below = if x_labels {
            (self.glyphs.line_metrics().line_height() + self.pad()).round()
        } else {
            0.0
        };
        Rect::from_edges(
            bounds.x0.round(),
            bounds.y0.round(),
            (bounds.x1 - right).round(),
            (bounds.y1 - below).round(),
        )
    }

    /// Draws the gridlines, the axis edges and the `y` labels of `plot`, and its `x` labels
    /// if `x_labels`. Call it before the series so they draw over the grid.
    pub fn paint_axes(&mut self, canvas: &mut Canvas<'_>, plot: &Plot, x_labels: bool) {
        let area = plot.area();
        if area.is_empty() {
            return;
        }
        let metrics = self.glyphs.line_metrics();
        let pad = self.pad();
        let palette = self.palette;

        let ticks = self.y_ticks(plot, 1);
        for value in ticks.clone() {
            // Lossless enough: pixels.
            let y = plot.y_at(value).round() as f32;
            if !(area.y0..area.y1).contains(&y) {
                continue;
            }
            canvas.fill_rect(Rect::new(area.x0, y, area.width(), 1.0), palette.grid);
            self.label.clear();
            if ticks.write_label(value, &mut self.label).is_ok() {
                let baseline = (y + (metrics.ascent + metrics.descent) / 2.0).round();
                let at = Point::new(area.x1 + pad, baseline);
                draw_text(
                    &mut self.glyphs,
                    canvas,
                    self.label.as_str(),
                    at,
                    palette.label,
                );
            }
        }

        let ticks = self.x_ticks(plot, 1);
        for value in ticks.clone() {
            // Lossless enough: pixels.
            let x = plot.x_at(value).round() as f32;
            if !(area.x0..area.x1).contains(&x) {
                continue;
            }
            canvas.fill_rect(Rect::new(x, area.y0, 1.0, area.height()), palette.grid);
            self.label.clear();
            if x_labels && ticks.write_label(value, &mut self.label).is_ok() {
                let width = self.glyphs.measure(self.label.as_str());
                let left = (x - width / 2.0).clamp(area.x0, (area.x1 - width).max(area.x0));
                let at = Point::new(left.round(), (area.y1 + pad + metrics.ascent).round());
                draw_text(
                    &mut self.glyphs,
                    canvas,
                    self.label.as_str(),
                    at,
                    palette.label,
                );
            }
        }

        let right = Rect::new(area.x1, area.y0, 1.0, area.height() + 1.0);
        canvas.fill_rect(right, palette.axis);
        canvas.fill_rect(Rect::new(area.x0, area.y1, area.width(), 1.0), palette.axis);
    }

    /// Draws a crosshair at `at` over `plot`: a line down the plot through `at`, and, if
    /// `at` is within the plot, a line across it with the `y` there in a box right of the
    /// plot; with `x_label`, the `x` there in a box under the plot. The values are written
    /// finer than the tick labels. For stacked plots, give each one the same `at` and label
    /// `x` under the lowest.
    pub fn paint_crosshair(
        &mut self,
        canvas: &mut Canvas<'_>,
        plot: &Plot,
        at: Point,
        x_label: bool,
    ) {
        let area = plot.area();
        let x = at.x.round();
        if area.is_empty() || !(area.x0..area.x1).contains(&x) {
            return;
        }
        let metrics = self.glyphs.line_metrics();
        let (pad, line) = (self.pad(), metrics.line_height().round());
        let palette = self.palette;
        canvas.fill_rect(Rect::new(x, area.y0, 1.0, area.height()), palette.crosshair);

        let y = at.y.round();
        if (area.y0..area.y1).contains(&y) {
            canvas.fill_rect(Rect::new(area.x0, y, area.width(), 1.0), palette.crosshair);
            let fine = self.y_ticks(plot, FINER);
            self.label.clear();
            if fine.write_label(plot.y_value(y), &mut self.label).is_ok() {
                let width = self.glyphs.measure(self.label.as_str());
                let top = (y - line / 2.0).round();
                let marker = Rect::new(area.x1 + 1.0, top, width + 2.0 * pad, line);
                canvas.fill_rect(marker, palette.marker);
                let baseline = (y + (metrics.ascent + metrics.descent) / 2.0).round();
                let at = Point::new(area.x1 + 1.0 + pad, baseline);
                draw_text(
                    &mut self.glyphs,
                    canvas,
                    self.label.as_str(),
                    at,
                    palette.marker_text,
                );
            }
        }

        if x_label {
            let fine = self.x_ticks(plot, FINER);
            self.label.clear();
            if fine.write_label(plot.x_value(x), &mut self.label).is_ok() {
                let width = self.glyphs.measure(self.label.as_str()) + 2.0 * pad;
                let left = (x - width / 2.0)
                    .clamp(area.x0, (area.x1 - width).max(area.x0))
                    .round();
                let marker = Rect::new(left, area.y1 + 1.0, width, pad + line);
                canvas.fill_rect(marker, palette.marker);
                let at = Point::new(left + pad, (area.y1 + pad + metrics.ascent).round());
                draw_text(
                    &mut self.glyphs,
                    canvas,
                    self.label.as_str(),
                    at,
                    palette.marker_text,
                );
            }
        }
    }

    /// The `y` ticks `plot` has room for, `finer` times as many.
    fn y_ticks(&self, plot: &Plot, finer: usize) -> Ticks {
        let line = self.glyphs.line_metrics().line_height();
        // Lossless enough: a whole number of ticks, saturating for absurd heights.
        let most = (plot.area().height() / (line * Y_TICK_LINES)).floor() as usize;
        plot.y().ticks(most.saturating_mul(finer))
    }

    /// The `x` ticks `plot` has room for, `finer` times as many: labels of times below a
    /// second are longer, and get more room.
    fn x_ticks(&mut self, plot: &Plot, finer: usize) -> Ticks {
        let (width, char_width) = (plot.area().width(), self.char_width());
        // Lossless enough: a whole number of ticks, saturating for absurd widths.
        let most =
            |room: f32| ((width / (char_width * room)).floor() as usize).saturating_mul(finer);
        let ticks = plot.x().ticks(most(X_LABEL_ROOM));
        if plot.x().kind() == AxisKind::Time && ticks.step() < 1.0 {
            return plot.x().ticks(most(X_FINE_LABEL_ROOM));
        }
        ticks
    }

    /// The advance of a digit.
    fn char_width(&mut self) -> f32 {
        let px = self.glyphs.px();
        self.glyphs.get('0').map_or(px * 0.6, |glyph| glyph.advance)
    }

    /// Space between the plot and its labels.
    fn pad(&self) -> f32 {
        (self.glyphs.px() * 0.5).round()
    }
}

/// Draws `text` with its pen starting at `at` (`y` is the baseline).
fn draw_text(glyphs: &mut GlyphMap, canvas: &mut Canvas<'_>, text: &str, at: Point, color: Color) {
    let mut pen = at.x;
    for ch in text.chars() {
        let Some(mapped) = glyphs.get(ch) else {
            continue;
        };
        let at = Point::new(pen, at.y);
        canvas.draw_glyph(glyphs.font(), mapped.id, glyphs.px(), at, color);
        pen += mapped.advance;
    }
}
