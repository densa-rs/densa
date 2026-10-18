//! Painting: parents under children under overlays, clipping, hidden widgets, culling.

mod common;

use common::{log, ui, Probe};
use densa_core::{Color, Rect};
use densa_draw::{Canvas, InstanceBuffer};
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{Overflow, PaintCx, Style, Ui, Widget};

/// The rectangles painted in one frame, and their colors.
fn frame(ui: &mut Ui) -> Vec<(Rect, [f32; 4])> {
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(256);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    ui.paint(&mut canvas);
    out.as_slice()
        .iter()
        .map(|i| {
            (
                Rect::new(i.rect[0], i.rect[1], i.rect[2], i.rect[3]),
                i.color,
            )
        })
        .collect()
}

fn gray(level: f32) -> Color {
    Color::linear(level, level, level, 1.0)
}

/// Paints a thin bar over its children.
struct Overlay;

impl Widget for Overlay {
    fn paint_over(&mut self, cx: &PaintCx<'_>, canvas: &mut densa_draw::Canvas<'_>) {
        let r = cx.rect();
        canvas.fill_rect(Rect::new(r.left(), r.top(), r.width(), 2.0), gray(0.9));
    }
}

#[test]
fn parents_paint_under_children_and_overlays_on_top() {
    let log = log();
    let mut ui = ui(200.0, 100.0);
    let panel = Probe::new("panel", &log).colored(gray(0.1));
    let panel = ui
        .add(ui.root(), panel, Style::column().size(100.0, 50.0))
        .unwrap();
    let overlay = ui.add(panel, Overlay, Style::column().grow(1.0)).unwrap();
    let child = Probe::new("child", &log).colored(gray(0.5));
    ui.add(overlay, child, Style::row().size(40.0, 20.0))
        .unwrap();
    let painted = frame(&mut ui);
    let colors: Vec<f32> = painted.iter().map(|(_, c)| c[0]).collect();
    // Root background, panel, child, then the overlay's bar.
    assert_eq!(colors[1..], [0.1, 0.5, 0.9]);
    assert!(!ui.needs_paint());
}

#[test]
fn clipping_widgets_cut_their_children_and_hidden_ones_are_not_painted() {
    let log = log();
    let mut ui = ui(200.0, 100.0);
    let frame_style = Style::column().size(50.0, 50.0).overflow(Overflow::Clip);
    let clip = ui
        .add(
            ui.root(),
            Probe::new("clip", &log).colored(gray(0.1)),
            frame_style,
        )
        .unwrap();
    let wide = Style::row().size(120.0, 20.0).shrink(0.0);
    ui.add(clip, Probe::new("wide", &log).colored(gray(0.5)), wide)
        .unwrap();
    let hidden = Style::row().size(10.0, 10.0).hidden(true);
    ui.add(
        ui.root(),
        Probe::new("hidden", &log).colored(gray(0.7)),
        hidden,
    )
    .unwrap();
    let painted = frame(&mut ui);
    let wide = painted.iter().find(|(_, c)| c[0] == 0.5).unwrap().0;
    assert_eq!(
        wide,
        Rect::new(0.0, 0.0, 50.0, 20.0),
        "cut at the frame's edge"
    );
    assert!(painted.iter().all(|(_, c)| c[0] != 0.7));
}

#[test]
fn widgets_scrolled_out_of_a_clipping_parent_are_skipped() {
    let log = log();
    let mut ui = ui(200.0, 100.0);
    let frame_style = Style::column().size(50.0, 50.0).overflow(Overflow::Clip);
    let clip = ui
        .add(ui.root(), Probe::new("clip", &log), frame_style)
        .unwrap();
    for _ in 0..20 {
        // Clipping rows: one outside the parent's clip is skipped with all it holds.
        let row = Style::row()
            .size(50.0, 25.0)
            .shrink(0.0)
            .overflow(Overflow::Clip);
        let row = ui
            .add(clip, Probe::new("row", &log).colored(gray(0.3)), row)
            .unwrap();
        ui.add(
            row,
            Probe::new("cell", &log).colored(gray(0.6)),
            Style::row().size(10.0, 10.0),
        )
        .unwrap();
    }
    frame(&mut ui);
    let painted: Vec<u32> = ui
        .children(clip)
        .iter()
        .map(|&row| ui.widget::<Probe>(row).unwrap().painted)
        .collect();
    let cells: u32 = ui
        .children(clip)
        .iter()
        .map(|&row| ui.widget::<Probe>(ui.children(row)[0]).unwrap().painted)
        .sum();
    assert_eq!(
        painted[..3],
        [1, 1, 0],
        "rows out of view are not even asked"
    );
    assert_eq!(painted.iter().sum::<u32>(), 2);
    assert_eq!(cells, 2, "nor are their cells");
}

#[test]
fn hidden_widgets_are_not_asked_to_paint() {
    let log = log();
    let mut ui = ui(200.0, 100.0);
    let hidden = Style::row().size(10.0, 10.0).hidden(true);
    let hidden = ui
        .add(ui.root(), Probe::new("hidden", &log), hidden)
        .unwrap();
    let inside = ui
        .add(
            hidden,
            Probe::new("inside", &log),
            Style::row().size(5.0, 5.0),
        )
        .unwrap();
    frame(&mut ui);
    assert_eq!(ui.widget::<Probe>(hidden).unwrap().painted, 0);
    assert_eq!(ui.widget::<Probe>(inside).unwrap().painted, 0);
}
