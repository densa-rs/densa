//! The chart widget: following new data, zooming and panning with the pointer and keys, the
//! crosshair, stacked panes and what screen readers hear.

use std::cell::RefCell;

use densa_chart::{paint_line, Axis, AxisKind, ChartModel, ChartPalette, LineStyle, Plot};
use densa_core::{Point, Size};
use densa_draw::{Canvas, Instance, InstanceBuffer};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{
    Action, ActionKind, Key, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerKind, Role, ScrollDelta, Theme, Ui, WidgetId,
};
use densa_widgets::Chart;

/// A line of numbers one apart, in two panes (3 to 1), recording the plots it is drawn on.
struct Line {
    points: Vec<(f64, f64)>,
    step: Option<f64>,
    plots: RefCell<Vec<(usize, Plot)>>,
    description: &'static str,
    weights: [f32; 2],
}

impl Line {
    fn new(count: u32) -> Self {
        Self {
            points: (0..count).map(|x| (f64::from(x), 5.0)).collect(),
            step: Some(1.0),
            plots: RefCell::new(Vec::new()),
            description: "",
            weights: [3.0, 1.0],
        }
    }
}

impl ChartModel for Line {
    fn x_kind(&self) -> AxisKind {
        AxisKind::Number
    }

    fn x_extent(&self) -> Option<(f64, f64)> {
        Some((self.points.first()?.0, self.points.last()?.0))
    }

    fn x_step(&self) -> Option<f64> {
        self.step
    }

    fn panes(&self) -> usize {
        2
    }

    fn pane_weight(&self, pane: usize) -> f32 {
        self.weights[pane]
    }

    fn y_axis(&self, _: usize, _: &Axis) -> Option<Axis> {
        Some(Axis::number(0.0, 10.0))
    }

    fn paint(&self, pane: usize, plot: &Plot, canvas: &mut Canvas<'_>, palette: &ChartPalette) {
        self.plots.borrow_mut().push((pane, *plot));
        paint_line(canvas, plot, &self.points[..], LineStyle::new(palette.line));
    }

    fn describe(&self) -> String {
        self.description.to_owned()
    }
}

type LineChart = Chart<Line>;

/// A 800 x 400 chart of `line` showing its last 100.
fn chart(line: Line) -> (Ui, WidgetId) {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(800.0, 400.0));
    let id = ui
        .add(ui.root(), LineChart::new(line, 100.0), LineChart::style())
        .unwrap();
    ui.layout();
    paint(&mut ui);
    (ui, id)
}

fn paint(ui: &mut Ui) -> Vec<Instance> {
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(65_536);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    ui.paint(&mut canvas);
    out.as_slice().to_vec()
}

fn of(ui: &Ui, id: WidgetId) -> &LineChart {
    ui.widget::<LineChart>(id).unwrap()
}

/// The shown range of `x`.
fn shown(ui: &Ui, id: WidgetId) -> (f64, f64) {
    let x = of(ui, id).x_axis().unwrap();
    (x.start(), x.end())
}

/// The plot of the last painted pane `pane`.
fn plot(ui: &Ui, id: WidgetId, pane: usize) -> Plot {
    let plots = of(ui, id).model().plots.borrow();
    plots.iter().rev().find(|(p, _)| *p == pane).unwrap().1
}

fn send(ui: &mut Ui, kind: PointerKind, at: Point) {
    ui.pointer(PointerEvent::new(kind, at));
}

fn wheel(ui: &mut Ui, dx: f32, dy: f32, at: Point) {
    send(ui, PointerKind::Scroll(ScrollDelta::Lines(dx, dy)), at);
}

fn drag(ui: &mut Ui, from: Point, to: Point) {
    send(ui, PointerKind::Down(PointerButton::Primary), from);
    send(ui, PointerKind::Move, to);
    send(ui, PointerKind::Up(PointerButton::Primary), to);
}

fn key(ui: &mut Ui, key: NamedKey) {
    ui.key(&KeyEvent::down(Key::Named(key)));
}

fn changes(ui: &mut Ui) -> usize {
    ui.actions()
        .filter(|Action { kind, .. }| *kind == ActionKind::Changed)
        .count()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

const MIDDLE: Point = Point::new(400.0, 150.0);

#[test]
fn it_shows_the_newest_span_and_moves_with_new_data() {
    let (mut ui, id) = chart(Line::new(1_001));
    assert!(of(&ui, id).is_following());
    let (start, end) = shown(&ui, id);
    assert!(near(end - start, 100.0));
    assert!(
        end > 1_000.0 && end < 1_005.0,
        "a little room after the newest: {end}"
    );
    ui.update::<LineChart, _>(id, |chart, cx| {
        chart.model_mut(cx).points.push((1_050.0, 5.0));
    });
    assert!(near(shown(&ui, id).1 - end, 50.0));
}

#[test]
fn without_data_there_is_nothing_to_show_or_move() {
    let (mut ui, id) = chart(Line::new(0));
    assert!(of(&ui, id).x_axis().is_none());
    wheel(&mut ui, 0.0, -1.0, MIDDLE);
    drag(&mut ui, MIDDLE, Point::new(100.0, 150.0));
    key(&mut ui, NamedKey::ArrowLeft);
    assert_eq!(changes(&mut ui), 0);
    let instances = paint(&mut ui);
    assert!(instances.iter().all(|i| !i.is_glyph()), "no axes");
    assert!(of(&ui, id).model().plots.borrow().is_empty());
}

#[test]
fn the_wheel_zooms_and_keeps_following_the_newest() {
    let (mut ui, id) = chart(Line::new(1_001));
    let (_, end) = shown(&ui, id);
    wheel(&mut ui, 0.0, -1.0, MIDDLE);
    let (start, new_end) = shown(&ui, id);
    assert!(
        near(new_end - start, 100.0 / 1.15),
        "zoomed in: {}",
        new_end - start
    );
    assert!(of(&ui, id).is_following());
    assert!(new_end < end && new_end > 1_000.0, "still at the newest");
    assert_eq!(changes(&mut ui), 1);
    wheel(&mut ui, 0.0, 2.0, MIDDLE);
    let (start, end) = shown(&ui, id);
    assert!(near(end - start, 100.0 * 1.15));
}

#[test]
fn zooming_stops_at_a_few_points_and_at_twice_the_data() {
    let (mut ui, id) = chart(Line::new(1_001));
    for _ in 0..100 {
        wheel(&mut ui, 0.0, -1.0, MIDDLE);
    }
    let (start, end) = shown(&ui, id);
    assert!(near(end - start, 5.0), "five steps: {}", end - start);
    let before = changes(&mut ui);
    wheel(&mut ui, 0.0, -1.0, MIDDLE);
    assert_eq!(changes(&mut ui), 0, "nothing left to zoom: {before}");
    for _ in 0..100 {
        wheel(&mut ui, 0.0, 1.0, MIDDLE);
    }
    let (start, end) = shown(&ui, id);
    assert!(
        near(end - start, 2_000.0),
        "twice the data: {}",
        end - start
    );
}

#[test]
fn without_a_step_the_narrowest_view_is_a_share_of_the_data() {
    let mut line = Line::new(1_001);
    line.step = None;
    let (mut ui, id) = chart(line);
    for _ in 0..200 {
        wheel(&mut ui, 0.0, -1.0, MIDDLE);
    }
    let (start, end) = shown(&ui, id);
    assert!(near(end - start, 0.1), "{}", end - start);
}

#[test]
fn dragging_pans_and_stops_following_until_it_reaches_the_newest() {
    let (mut ui, id) = chart(Line::new(1_001));
    let (start, end) = shown(&ui, id);
    let width = f64::from(plot(&ui, id, 1).area().width());
    drag(&mut ui, MIDDLE, Point::new(500.0, 150.0));
    let (moved_start, moved_end) = shown(&ui, id);
    let delta = 100.0 / width * 100.0;
    assert!(near(moved_start, start - delta) && near(moved_end, end - delta));
    assert!(!of(&ui, id).is_following());
    assert_eq!(changes(&mut ui), 1);

    // New data does not move a panned view.
    ui.update::<LineChart, _>(id, |chart, cx| {
        chart.model_mut(cx).points.push((1_050.0, 5.0));
    });
    assert!(near(shown(&ui, id).0, moved_start));

    // Dragging back past the newest follows again.
    drag(&mut ui, MIDDLE, Point::new(100.0, 150.0));
    assert!(!of(&ui, id).is_following());
    drag(&mut ui, MIDDLE, Point::new(100.0, 150.0));
    assert!(of(&ui, id).is_following());
    assert!(
        near(shown(&ui, id).1 - 1_050.0, 3.0),
        "{:?}",
        shown(&ui, id)
    );
}

#[test]
fn a_panned_view_zooms_around_the_pointer() {
    let (mut ui, id) = chart(Line::new(1_001));
    drag(&mut ui, MIDDLE, Point::new(700.0, 150.0));
    paint(&mut ui);
    let at = Point::new(200.0, 150.0);
    let before = plot(&ui, id, 0).x_value(at.x);
    wheel(&mut ui, 0.0, -1.0, at);
    paint(&mut ui);
    let after = plot(&ui, id, 0).x_value(at.x);
    assert!((before - after).abs() < 1e-6, "{before} moved to {after}");
    let (start, end) = shown(&ui, id);
    assert!(near(end - start, 100.0 / 1.15));
}

#[test]
fn drags_one_after_another_from_the_same_spot_keep_panning_and_stay_over_the_data() {
    let (mut ui, id) = chart(Line::new(1_001));
    for _ in 0..50 {
        drag(&mut ui, Point::new(100.0, 150.0), Point::new(700.0, 150.0));
    }
    let (start, end) = shown(&ui, id);
    assert!(near(end, 0.0), "ends at the oldest: {start}..{end}");
}

#[test]
fn a_double_click_follows_again() {
    let (mut ui, id) = chart(Line::new(1_001));
    drag(&mut ui, MIDDLE, Point::new(600.0, 150.0));
    assert!(!of(&ui, id).is_following());
    let _ = changes(&mut ui);
    for kind in [
        PointerKind::Down(PointerButton::Primary),
        PointerKind::Up(PointerButton::Primary),
        PointerKind::Down(PointerButton::Primary),
        PointerKind::Up(PointerButton::Primary),
    ] {
        send(&mut ui, kind, MIDDLE);
    }
    assert!(of(&ui, id).is_following());
    assert_eq!(changes(&mut ui), 1);
}

#[test]
fn sideways_and_shifted_scrolls_pan() {
    let (mut ui, id) = chart(Line::new(1_001));
    let (start, _) = shown(&ui, id);
    wheel(&mut ui, -1.0, 0.0, MIDDLE);
    let (sideways, _) = shown(&ui, id);
    assert!(sideways < start);
    let shifted = PointerEvent::new(PointerKind::Scroll(ScrollDelta::Lines(0.0, -1.0)), MIDDLE)
        .with_modifiers(Modifiers::SHIFT);
    ui.pointer(shifted);
    let (further, end) = shown(&ui, id);
    assert!(further < sideways);
    assert!(near(end - further, 100.0), "panning keeps the width");
    let touchpad = PointerKind::Scroll(ScrollDelta::Pixels(30.0, 0.0));
    send(&mut ui, touchpad, MIDDLE);
    assert!(shown(&ui, id).0 > further);
}

#[test]
fn keys_pan_zoom_and_jump_to_either_end() {
    let (mut ui, id) = chart(Line::new(1_001));
    send(&mut ui, PointerKind::Down(PointerButton::Primary), MIDDLE);
    send(&mut ui, PointerKind::Up(PointerButton::Primary), MIDDLE);
    let (start, _) = shown(&ui, id);
    key(&mut ui, NamedKey::ArrowLeft);
    assert!(near(shown(&ui, id).0, start - 10.0));
    assert!(!of(&ui, id).is_following());
    key(&mut ui, NamedKey::ArrowRight);
    assert!(of(&ui, id).is_following(), "back at the newest");
    key(&mut ui, NamedKey::ArrowUp);
    let (start, end) = shown(&ui, id);
    assert!(near(end - start, 100.0 / 1.15));
    key(&mut ui, NamedKey::ArrowDown);
    let (start, end) = shown(&ui, id);
    assert!(near(end - start, 100.0));
    key(&mut ui, NamedKey::Home);
    assert!(near(shown(&ui, id).0, 0.0));
    key(&mut ui, NamedKey::End);
    assert!(of(&ui, id).is_following());
    let _ = changes(&mut ui);
    ui.key(&KeyEvent::down(Key::Named(NamedKey::ArrowLeft)).with_modifiers(Modifiers::CONTROL));
    assert!(
        of(&ui, id).is_following(),
        "modified keys are someone else's"
    );
    assert_eq!(changes(&mut ui), 0);
}

#[test]
fn show_picks_a_range_within_the_limits() {
    let (mut ui, id) = chart(Line::new(1_001));
    ui.update::<LineChart, _>(id, |chart, cx| chart.show(cx, Axis::number(200.0, 300.0)));
    assert_eq!(shown(&ui, id), (200.0, 300.0));
    assert!(!of(&ui, id).is_following());
    ui.update::<LineChart, _>(id, |chart, cx| chart.show(cx, Axis::number(500.0, 501.0)));
    let (start, end) = shown(&ui, id);
    assert!(
        near(start, 498.0) && near(end, 503.0),
        "five steps around it"
    );
    ui.update::<LineChart, _>(id, |chart, cx| chart.show(cx, Axis::number(950.0, 1_100.0)));
    assert!(of(&ui, id).is_following());
    ui.update::<LineChart, _>(id, |chart, cx| chart.follow(cx));
    assert!(of(&ui, id).is_following());
}

#[test]
fn panes_share_the_height_by_weight_and_the_lowest_has_the_time_labels() {
    let (ui, id) = chart(Line::new(1_001));
    let top = plot(&ui, id, 0).area();
    let bottom = plot(&ui, id, 1).area();
    assert_eq!(top.left(), bottom.left());
    assert_eq!(top.right(), bottom.right());
    assert!(top.bottom() <= bottom.top());
    assert_eq!(top.top(), 0.0);
    // 300 and 100 pixels, the lower one less a line for the labels.
    assert!((top.height() - 300.0).abs() <= 1.0, "{top:?}");
    assert!(
        bottom.height() < 90.0 && bottom.height() > 70.0,
        "{bottom:?}"
    );
    assert_eq!(plot(&ui, id, 0).x(), plot(&ui, id, 1).x());
}

#[test]
fn the_crosshair_shows_while_the_pointer_is_over_the_chart() {
    let (mut ui, id) = chart(Line::new(1_001));
    let marker = Theme::dark().raised.to_array();
    let markers = |instances: &[Instance]| instances.iter().filter(|i| i.color == marker).count();
    assert_eq!(markers(&paint(&mut ui)), 0);
    send(&mut ui, PointerKind::Move, MIDDLE);
    // The y value right of the top pane, the x value under the bottom one.
    assert_eq!(markers(&paint(&mut ui)), 2);
    send(&mut ui, PointerKind::Down(PointerButton::Primary), MIDDLE);
    assert_eq!(markers(&paint(&mut ui)), 0, "not while dragging");
    send(&mut ui, PointerKind::Up(PointerButton::Primary), MIDDLE);
    ui.pointer_left(MIDDLE);
    assert_eq!(markers(&paint(&mut ui)), 0);
    let _ = id;
}

#[test]
fn screen_readers_hear_the_models_description() {
    let mut line = Line::new(10);
    line.description = "Test line, last 5";
    let (mut ui, id) = chart(line);
    let tree = ui.access_tree();
    let node = tree.node(ui.access_id(id).unwrap()).unwrap();
    assert_eq!(node.role(), Role::Image);
    assert_eq!(node.label(), Some("Test line, last 5"));

    let (mut ui, id) = chart(Line::new(10));
    let tree = ui.access_tree();
    let node = tree.node(ui.access_id(id).unwrap()).unwrap();
    assert_eq!(node.label(), Some("Chart"));
}

#[test]
fn a_span_that_is_not_a_width_shows_one() {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(800.0, 400.0));
    let id = ui
        .add(
            ui.root(),
            LineChart::new(Line::new(100), f64::NAN),
            LineChart::style(),
        )
        .unwrap();
    ui.layout();
    let (start, end) = shown(&ui, id);
    assert!(near(end - start, 1.0));
}

#[test]
fn the_crosshair_is_where_the_drag_let_go() {
    let (mut ui, _id) = chart(Line::new(1_001));
    let vertical = |instances: &[Instance]| -> Vec<f32> {
        let line = Theme::dark().text_dim.with_alpha(0.7).to_array();
        instances
            .iter()
            .filter(|i| i.color == line && i.rect[2] == 1.0)
            .map(|i| i.rect[0])
            .collect()
    };
    send(&mut ui, PointerKind::Move, Point::new(200.0, 150.0));
    drag(&mut ui, Point::new(200.0, 150.0), Point::new(600.0, 150.0));
    let lines = vertical(&paint(&mut ui));
    assert!(
        !lines.is_empty() && lines.iter().all(|&x| x == 600.0),
        "{lines:?}"
    );
}

#[test]
fn one_point_without_a_step_zooms_within_sane_limits() {
    let mut line = Line::new(0);
    line.points = vec![(1.79e9, 5.0)];
    line.step = None;
    let (mut ui, id) = chart(line);
    for _ in 0..400 {
        wheel(&mut ui, 0.0, -1.0, MIDDLE);
    }
    let (start, end) = shown(&ui, id);
    assert!(end - start >= 1.79e9 * 1e-12 * 0.99, "{}", end - start);
    assert!(end - start < 1.0, "zoomed in: {}", end - start);
    for _ in 0..400 {
        wheel(&mut ui, 0.0, 1.0, MIDDLE);
    }
    let (start, end) = shown(&ui, id);
    assert!(
        near(end - start, 100.0),
        "back out to the first view: {}",
        end - start
    );
}

#[test]
fn a_panned_view_stays_over_data_that_was_trimmed() {
    let (mut ui, id) = chart(Line::new(1_001));
    ui.update::<LineChart, _>(id, |chart, cx| chart.show(cx, Axis::number(100.0, 200.0)));
    ui.update::<LineChart, _>(id, |chart, cx| {
        chart.model_mut(cx).points.drain(..500);
    });
    paint(&mut ui);
    let (start, end) = shown(&ui, id);
    assert!(near(start, 400.0) && near(end, 500.0), "{start}..{end}");
    assert!(!of(&ui, id).is_following());
}

#[test]
fn room_for_wider_labels_narrows_the_plots() {
    let (ui, id) = chart(Line::new(1_001));
    let narrow = plot(&ui, id, 0).area().width();
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(800.0, 400.0));
    let wide = LineChart::new(Line::new(1_001), 100.0).with_label_chars(14);
    let wide = ui.add(ui.root(), wide, LineChart::style()).unwrap();
    ui.layout();
    paint(&mut ui);
    assert!(plot(&ui, wide, 0).area().width() < narrow - 40.0);
}

#[test]
fn a_pane_weight_that_is_not_a_number_counts_as_none() {
    for weight in [f32::INFINITY, f32::NAN, -1.0] {
        let mut line = Line::new(1_001);
        line.weights = [weight, 1.0];
        let (ui, id) = chart(line);
        let bottom = plot(&ui, id, 1).area();
        assert!(bottom.height() > 300.0, "{weight}: {bottom:?}");
        let plots = of(&ui, id).model().plots.borrow();
        assert!(plots
            .iter()
            .all(|(_, plot)| plot.area().height().is_finite()));
    }
}
