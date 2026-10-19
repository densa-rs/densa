//! A chart over a live table: one symbol's trades, a view filtered by symbol and sorted by
//! time, kept up as trades arrive and switched when another symbol is chosen.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use densa_arrow::LiveSeries;
use densa_chart::{
    paint_line, points_within, y_extent, Axis, AxisKind, ChartModel, ChartPalette, LineStyle, Plot,
};
use densa_core::Size;
use densa_data::{CompareOp, Filter, LiveTable, Order, Scalar, ViewId, ViewSpec};
use densa_draw::{Canvas, InstanceBuffer};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{Ui, WidgetId};
use densa_widgets::Chart;

/// Trades: `(id, symbol, time, price)`.
fn trades(rows: &[(i64, &str, f64, f64)]) -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "id",
            Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.0))) as ArrayRef,
        ),
        (
            "symbol",
            Arc::new(StringArray::from_iter_values(rows.iter().map(|r| r.1))),
        ),
        (
            "time",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.2))),
        ),
        (
            "price",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.3))),
        ),
    ])
    .unwrap()
}

/// One symbol's trades as a price line.
struct Trail {
    trades: LiveTable,
    price: LiveSeries,
    view: ViewId,
}

impl Trail {
    fn new(mut trades: LiveTable, symbol: &str) -> Self {
        let price = LiveSeries::new(&trades, "time", "price").unwrap();
        let view = trades.add_view(spec(symbol)).unwrap();
        Self {
            trades,
            price,
            view,
        }
    }

    fn show(&mut self, symbol: &str) {
        let old = self.view;
        self.view = self.trades.add_view(spec(symbol)).unwrap();
        self.trades.remove_view(old);
    }
}

/// `symbol`'s trades by time.
fn spec(symbol: &str) -> ViewSpec {
    let only = Filter::compare("symbol", CompareOp::Eq, Scalar::Str(symbol.to_owned()));
    ViewSpec::new()
        .filter(only)
        .sort_by("time", Order::Ascending)
}

impl ChartModel for Trail {
    fn x_kind(&self) -> AxisKind {
        AxisKind::Number
    }

    fn x_extent(&self) -> Option<(f64, f64)> {
        self.price.points(&self.trades, self.view)?.x_extent()
    }

    fn y_axis(&self, _: usize, x: &Axis) -> Option<Axis> {
        let points = self.price.points(&self.trades, self.view)?;
        let (low, high) = y_extent(&points, points_within(&points, x))?;
        Some(Axis::number(low, high))
    }

    fn paint(&self, _: usize, plot: &Plot, canvas: &mut Canvas<'_>, palette: &ChartPalette) {
        if let Some(points) = self.price.points(&self.trades, self.view) {
            paint_line(canvas, plot, &points, LineStyle::new(palette.line));
        }
    }

    fn prepare(&mut self) {
        self.price.sync(&self.trades);
    }
}

type TrailChart = Chart<Trail>;

/// A chart of A's trades among B's, showing 100 seconds.
fn chart() -> (Ui, WidgetId) {
    let first = trades(&[
        (1, "A", 10.0, 1.0),
        (2, "B", 11.0, 100.0),
        (3, "A", 20.0, 2.0),
        (4, "B", 21.0, 200.0),
        (5, "A", 30.0, 3.0),
    ]);
    let mut table = LiveTable::new(first.schema(), "id").unwrap();
    table.upsert(first).unwrap();
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(600.0, 300.0));
    let chart = TrailChart::new(Trail::new(table, "A"), 100.0);
    let id = ui.add(ui.root(), chart, TrailChart::style()).unwrap();
    ui.layout();
    paint(&mut ui);
    (ui, id)
}

fn paint(ui: &mut Ui) {
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(65_536);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    ui.paint(&mut canvas);
}

/// The chart's shown range of `x`, and the range of `y` its model gives for it.
fn shown(ui: &Ui, id: WidgetId) -> ((f64, f64), Option<(f64, f64)>) {
    let chart = ui.widget::<TrailChart>(id).unwrap();
    let x = chart.x_axis().unwrap();
    let y = chart.model().y_axis(0, &x).map(|y| (y.start(), y.end()));
    ((x.start(), x.end()), y)
}

#[test]
fn the_chart_shows_one_symbols_trades_and_follows_new_ones() {
    let (mut ui, id) = chart();
    let ((_, end), y) = shown(&ui, id);
    assert!(end > 30.0 && end < 35.0, "the newest of A's: {end}");
    assert_eq!(y, Some((1.0, 3.0)));

    ui.update::<TrailChart, _>(id, |chart, cx| {
        let trades = trades(&[(6, "B", 40.0, 400.0), (7, "A", 50.0, 5.0)]);
        chart.model_mut(cx).trades.upsert(trades).unwrap();
    });
    // Not taken in until painted: no values from batches the series has not seen.
    assert!(ui.widget::<TrailChart>(id).unwrap().x_axis().is_none());
    paint(&mut ui);
    let ((_, end), y) = shown(&ui, id);
    assert!(end > 50.0 && end < 55.0, "{end}");
    assert_eq!(y, Some((1.0, 5.0)));
}

#[test]
fn another_symbol_shows_its_own_trades() {
    let (mut ui, id) = chart();
    ui.update::<TrailChart, _>(id, |chart, cx| {
        chart.model_mut(cx).show("B");
        chart.follow(cx);
    });
    paint(&mut ui);
    let ((_, end), y) = shown(&ui, id);
    assert!(end > 21.0 && end < 25.0, "{end}");
    assert_eq!(y, Some((100.0, 200.0)));
}

#[test]
fn deleted_and_compacted_trades_leave_the_chart() {
    let (mut ui, id) = chart();
    ui.update::<TrailChart, _>(id, |chart, cx| {
        let model = chart.model_mut(cx);
        model.trades.delete(&Int64Array::from(vec![1, 2])).unwrap();
        model.trades.compact().unwrap();
    });
    paint(&mut ui);
    assert_eq!(shown(&ui, id).1, Some((2.0, 3.0)));
}
