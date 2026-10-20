use densa_draw::Canvas;

use crate::{Axis, AxisKind, ChartPalette, Plot};

/// The data of a chart widget (`densa_widgets::Chart`): panes stacked top to bottom on one
/// horizontal axis, each drawing its own series (candles over a line, volume bars below).
///
/// The widget decides what part of `x` shows (it follows the newest data until the user
/// pans or zooms) and asks each pane for its vertical axis over that part, so panes scale to
/// what is on screen. Painting goes through the series functions of this crate, which
/// allocate nothing.
pub trait ChartModel {
    /// What the horizontal axis counts.
    fn x_kind(&self) -> AxisKind {
        AxisKind::Time
    }

    /// The lowest and highest `x` of the data, or `None` while there is none.
    fn x_extent(&self) -> Option<(f64, f64)>;

    /// The distance between two points (a candle's period), if the data has one: the
    /// widget zooms in no further than a few of them.
    fn x_step(&self) -> Option<f64> {
        None
    }

    /// How many panes, top to bottom (one is drawn even for zero).
    fn panes(&self) -> usize {
        1
    }

    /// Pane `pane`'s share of the height, against the others' (above zero).
    fn pane_weight(&self, pane: usize) -> f32 {
        let _ = pane;
        1.0
    }

    /// The vertical axis of pane `pane` while `x` shows: what its series need, with room
    /// around them (see [`Axis::padded`]); `None` if it has nothing there.
    fn y_axis(&self, pane: usize, x: &Axis) -> Option<Axis>;

    /// Draws the series of pane `pane` onto `plot`, in `palette`'s colors.
    fn paint(&self, pane: usize, plot: &Plot, canvas: &mut Canvas<'_>, palette: &ChartPalette);

    /// The name of pane `pane`, written in its top left corner over its series ("Volume");
    /// nothing is written for an empty name, the default.
    fn pane_title(&self, pane: usize) -> &str {
        let _ = pane;
        ""
    }

    /// Called before every frame is painted, to catch up with new data. Does nothing by
    /// default.
    fn prepare(&mut self) {}

    /// What the chart shows, for screen readers ("ACME, one-minute candles, last 182.40").
    /// Asked only while one is listening. Empty by default.
    fn describe(&self) -> String {
        String::new()
    }
}
