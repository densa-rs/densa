//! Category axes: names under their places, cut to their room, and in the crosshair's box.

use std::cell::RefCell;

use densa_chart::{Axis, ChartPainter, ChartPalette, Plot, Point, Rect};
use densa_draw::{Canvas, Instance, InstanceBuffer};
use densa_text::{AtlasConfig, Font, GlyphCache};

const NAMES: [&str; 4] = ["north", "south", "east", "west"];

/// What `draw` puts on a fresh canvas.
fn drawn(draw: impl FnOnce(&mut Canvas<'_>)) -> Vec<Instance> {
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let mut out = InstanceBuffer::with_capacity(100_000);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    draw(&mut canvas);
    assert_eq!(out.dropped(), 0);
    out.as_slice().to_vec()
}

fn painter() -> ChartPainter {
    ChartPainter::new(Font::bundled_mono().unwrap(), 13.0, ChartPalette::new())
}

/// Four places across a 400-pixel-wide plot area, with room for labels below.
fn four(painter: &mut ChartPainter) -> Plot {
    let area = painter.plot_area(Rect::new(0.0, 0.0, 460.0, 300.0), true);
    Plot::new(area, Axis::category(-0.5, 3.5), Axis::number(0.0, 10.0))
}

/// The glyphs below `plot`, by left edge.
fn below(instances: &[Instance], plot: &Plot) -> Vec<[f32; 4]> {
    let mut glyphs: Vec<[f32; 4]> = instances
        .iter()
        .filter(|i| i.is_glyph() && i.rect[1] >= plot.area().y1)
        .map(|i| i.rect)
        .collect();
    glyphs.sort_by(|a, b| a[0].total_cmp(&b[0]));
    glyphs
}

/// How many of `glyphs` lie within `half` pixels of `x`, and whether all of those do whole.
fn around(glyphs: &[[f32; 4]], x: f32, half: f32) -> usize {
    glyphs
        .iter()
        .filter(|g| g[0] < x + half && g[0] + g[2] > x - half)
        .inspect(|g| assert!(g[0] >= x - half && g[0] + g[2] <= x + half, "{g:?} by {x}"))
        .count()
}

#[test]
fn each_place_is_labelled_with_its_name_under_it() {
    let mut painter = painter();
    let plot = four(&mut painter);
    let names = |i: usize| NAMES.get(i).copied();
    let instances = drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &names));

    let glyphs = below(&instances, &plot);
    assert_eq!(glyphs.len(), 18, "every letter of every name: {glyphs:?}");
    let gap = plot.x_px(1.0) - plot.x_px(0.0);
    for (place, name) in NAMES.iter().enumerate() {
        let x = plot.x_px(place as f64);
        assert_eq!(
            around(&glyphs, x, gap / 2.0),
            name.len(),
            "{name} centred at {x}"
        );
    }

    let unlabelled = drawn(|canvas| painter.paint_axes_named(canvas, &plot, false, &names));
    assert!(
        below(&unlabelled, &plot).is_empty(),
        "no x labels asked for"
    );
    let unnamed = drawn(|canvas| painter.paint_axes(canvas, &plot, true));
    assert!(below(&unnamed, &plot).is_empty(), "no names given");
}

#[test]
fn a_name_longer_than_its_room_is_cut_short_of_the_next() {
    let mut painter = painter();
    let plot = four(&mut painter);
    let long = "a category name far too long to fit";
    let names = |i: usize| (i < 4).then_some(long);
    let instances = drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &names));

    let glyphs = below(&instances, &plot);
    let gap = plot.x_px(1.0) - plot.x_px(0.0);
    let letters = long.chars().filter(|c| *c != ' ').count();
    for place in 0..4 {
        let x = plot.x_px(f64::from(place));
        let shown = around(&glyphs, x, gap / 2.0);
        assert!(
            shown > 3 && shown < letters,
            "cut at place {place}: {shown}"
        );
    }
    let shown = around(&glyphs, plot.x_px(0.0), gap / 2.0);
    assert!(
        glyphs.len() >= 4 * shown - 4,
        "each place gets about the same room"
    );
}

#[test]
fn names_are_asked_for_places_on_the_axis_only() {
    let mut painter = painter();
    // Room for a label at every place.
    let area = painter.plot_area(Rect::new(0.0, 0.0, 800.0, 300.0), true);
    let plot = Plot::new(area, Axis::category(-3.5, 3.5), Axis::number(0.0, 10.0));
    let asked = RefCell::new(Vec::new());
    let names = |i: usize| {
        asked.borrow_mut().push(i);
        NAMES.get(i).copied()
    };
    let instances = drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &names));
    assert_eq!(
        *asked.borrow(),
        [0, 1, 2, 3],
        "not for the ticks left of zero"
    );
    assert_eq!(below(&instances, &plot).len(), 18);
}

#[test]
fn many_places_are_labelled_every_few() {
    let mut painter = painter();
    let area = painter.plot_area(Rect::new(0.0, 0.0, 460.0, 300.0), true);
    let plot = Plot::new(area, Axis::category(-0.5, 99.5), Axis::number(0.0, 10.0));
    let asked = RefCell::new(Vec::new());
    let names = |i: usize| {
        asked.borrow_mut().push(i);
        Some("x")
    };
    drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &names));
    let asked = asked.into_inner();
    assert!(asked.len() >= 2 && asked.len() < 12, "{asked:?}");
    assert!(asked.windows(2).all(|w| w[1] - w[0] == asked[1] - asked[0]));
}

#[test]
fn the_crosshair_names_the_nearest_place() {
    let mut painter = painter();
    let palette = *painter.palette();
    let plot = four(&mut painter);
    let names = |i: usize| NAMES.get(i).copied();
    // A little right of place 2 ("east").
    let x = plot.x_px(2.2);
    let at = Point::new(x, plot.area().y0 + 20.0);
    let instances = drawn(|canvas| painter.paint_crosshair_named(canvas, &plot, at, true, &names));

    let marker = palette.marker.to_array();
    let boxes: Vec<&Instance> = instances
        .iter()
        .filter(|i| i.color == marker && i.rect[1] > plot.area().y1)
        .collect();
    assert_eq!(boxes.len(), 1, "a box below the plot");
    let text = palette.marker_text.to_array();
    let written: Vec<&Instance> = instances
        .iter()
        .filter(|i| i.is_glyph() && i.color == text && i.rect[1] > plot.area().y1)
        .collect();
    assert_eq!(written.len(), 4, "east");
    let [left, _, width, _] = boxes[0].rect;
    assert!(written
        .iter()
        .all(|g| g.rect[0] >= left && g.rect[0] + g.rect[2] <= left + width));

    // Past the last place there is no name, and nothing is written below.
    let past = Point::new(plot.area().x1 - 1.0, at.y);
    let nothing = drawn(|canvas| {
        painter.paint_crosshair_named(canvas, &plot, past, true, &|i| (i < 3).then_some("x"))
    });
    assert!(nothing.iter().all(|i| i.rect[1] <= plot.area().y1));
    let unnamed = drawn(|canvas| painter.paint_crosshair(canvas, &plot, at, true));
    assert!(unnamed.iter().all(|i| i.rect[1] <= plot.area().y1));
}

#[test]
fn a_name_wider_than_the_plot_keeps_its_crosshair_box_inside() {
    let mut painter = painter();
    let area = painter.plot_area(Rect::new(0.0, 0.0, 160.0, 100.0), true);
    let plot = Plot::new(area, Axis::category(-0.5, 0.5), Axis::number(0.0, 1.0));
    let long = "a category name far too long to fit under this plot";
    let at = Point::new(plot.x_px(0.0), area.y0 + 10.0);
    let instances =
        drawn(|canvas| painter.paint_crosshair_named(canvas, &plot, at, true, &|_| Some(long)));
    let under: Vec<&Instance> = instances.iter().filter(|i| i.rect[1] > area.y1).collect();
    assert!(!under.is_empty());
    assert!(under
        .iter()
        .all(|i| i.rect[0] >= area.x0 && i.rect[0] + i.rect[2] <= area.x1 + 0.5));
}

#[test]
fn labels_near_the_edges_keep_out_of_their_neighbours_room() {
    let mut painter = painter();
    let area = painter.plot_area(Rect::new(0.0, 0.0, 460.0, 300.0), true);
    // Panned so places 0 and 3 sit right at the plot's edges.
    let plot = Plot::new(area, Axis::category(0.0, 3.01), Axis::number(0.0, 10.0));
    let long = "a category name far too long to fit";
    let instances = drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &|_| Some(long)));
    let glyphs = below(&instances, &plot);
    let slot = plot.x_px(1.0) - plot.x_px(0.0);
    for place in 0..4 {
        let x = plot.x_px(f64::from(place));
        let mine = glyphs
            .iter()
            .filter(|g| (g[0] + g[2] / 2.0 - x).abs() < slot / 2.0);
        for g in mine {
            assert!(g[0] >= area.x0 && g[0] + g[2] <= area.x1 + 0.5, "{g:?}");
            assert!(
                g[0] >= x - slot / 2.0 && g[0] + g[2] <= x + slot / 2.0,
                "{g:?} by {x}"
            );
        }
    }
    assert!(
        glyphs.len() > 4 * 2,
        "the edge ones still write something: {}",
        glyphs.len()
    );
}

#[test]
fn a_cut_label_is_centred_under_its_place() {
    let mut painter = painter();
    let plot = four(&mut painter);
    let long = "a category name far too long to fit";
    let instances = drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &|_| Some(long)));
    let glyphs = below(&instances, &plot);
    let x = plot.x_px(1.0);
    let half = (plot.x_px(1.0) - plot.x_px(0.0)) / 2.0;
    let mine: Vec<_> = glyphs.iter().filter(|g| (g[0] - x).abs() < half).collect();
    let left = mine.first().unwrap()[0];
    let right = mine.last().map(|g| g[0] + g[2]).unwrap();
    let centre = (left + right) / 2.0;
    assert!((centre - x).abs() < 5.0, "{centre} under {x}");
}

#[test]
fn line_breaks_and_tabs_in_a_name_are_not_drawn() {
    let mut painter = painter();
    let plot = four(&mut painter);
    let plain = drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &|_| Some("ab")));
    let broken =
        drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &|_| Some("a\n\tb\r")));
    assert_eq!(below(&broken, &plot), below(&plain, &plot));
}

#[test]
fn a_place_too_narrow_for_even_an_ellipsis_has_no_label() {
    let mut painter = painter();
    let area = painter.plot_area(Rect::new(0.0, 0.0, 460.0, 300.0), true);
    // A hundred places on screen, every one asked: a few pixels each.
    let plot = Plot::new(area, Axis::category(-0.5, 99.5), Axis::number(0.0, 10.0));
    let instances = drawn(|canvas| {
        painter.paint_crosshair_named(
            canvas,
            &Plot::new(Rect::new(0.0, 0.0, 6.0, 50.0), plot.x(), plot.y()),
            Point::new(3.0, 10.0),
            true,
            &|_| Some("north"),
        )
    });
    assert!(
        instances.iter().all(|i| !i.is_glyph() || i.rect[1] < 50.0),
        "{instances:?}"
    );
}

/// The left and right ends of the glyphs within half a slot of `x`.
fn span_around(glyphs: &[[f32; 4]], x: f32, half: f32) -> (f32, f32) {
    let mine: Vec<_> = glyphs
        .iter()
        .filter(|g| (g[0] + g[2] / 2.0 - x).abs() < half)
        .collect();
    let left = mine.iter().map(|g| g[0]).fold(f32::MAX, f32::min);
    let right = mine.iter().map(|g| g[0] + g[2]).fold(f32::MIN, f32::max);
    (left, right)
}

#[test]
fn a_short_name_is_centred_and_cut_names_leave_a_gap_between_them() {
    let mut painter = painter();
    let plot = four(&mut painter);
    let slot = plot.x_px(1.0) - plot.x_px(0.0);
    let short = drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &|_| Some("mmmm")));
    let (left, right) = span_around(&below(&short, &plot), plot.x_px(1.0), slot / 2.0);
    assert!(
        ((left + right) / 2.0 - plot.x_px(1.0)).abs() < 2.0,
        "{left}..{right}"
    );

    let long = "mmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm";
    let cut = drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &|_| Some(long)));
    let glyphs = below(&cut, &plot);
    let (_, first_ends) = span_around(&glyphs, plot.x_px(1.0), slot / 2.0);
    let (second_starts, _) = span_around(&glyphs, plot.x_px(2.0), slot / 2.0);
    assert!(
        second_starts - first_ends >= 4.0,
        "{first_ends} then {second_starts}"
    );
}

#[test]
fn a_cut_name_ends_in_an_ellipsis() {
    let mut painter = painter();
    let plot = four(&mut painter);
    let long = "mmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm";
    let instances = drawn(|canvas| painter.paint_axes_named(canvas, &plot, true, &|_| Some(long)));
    let glyphs = below(&instances, &plot);
    // An m is about as tall as it is wide; the dots of an ellipsis are half as tall at most.
    let dots = glyphs.iter().filter(|g| g[3] <= g[2] / 2.0).count();
    assert_eq!(dots, 4, "one at the end of each name: {glyphs:?}");
}

#[test]
fn the_crosshair_names_nothing_without_x_labels_and_its_box_holds_a_cut_name() {
    let mut painter = painter();
    let palette = *painter.palette();
    let plot = four(&mut painter);
    let at = Point::new(plot.x_px(1.0), plot.area().y0 + 20.0);
    let names = |_| Some("north");
    let unlabelled =
        drawn(|canvas| painter.paint_crosshair_named(canvas, &plot, at, false, &names));
    assert!(unlabelled.iter().all(|i| i.rect[1] <= plot.area().y1));

    let narrow = Plot::new(Rect::new(0.0, 0.0, 40.0, 100.0), plot.x(), plot.y());
    let at = Point::new(20.0, 10.0);
    let long = |_| Some("mmmmmmmmmmmmmmmmmmmmmmmmmmmmmm");
    let instances = drawn(|canvas| painter.paint_crosshair_named(canvas, &narrow, at, true, &long));
    let marker = palette.marker.to_array();
    let boxed = instances
        .iter()
        .find(|i| i.color == marker && i.rect[1] > 100.0)
        .unwrap();
    let [left, _, width, _] = boxed.rect;
    let pad = 7.0;
    let text: Vec<_> = instances
        .iter()
        .filter(|i| i.is_glyph() && i.rect[1] > 100.0)
        .collect();
    let end = text
        .iter()
        .map(|g| g.rect[0] + g.rect[2])
        .fold(0.0, f32::max);
    assert!(
        end <= left + width && end >= left + width - pad - 4.0,
        "{end} in {boxed:?}"
    );
}
