//! Layout: flexbox sizes and places widgets, leaves are measured, and the pass runs only when
//! something changed.

mod common;

use common::{log, ui, Probe};
use densa_core::{Rect, Size};
use densa_ui::{Align, Edges, Length, Style};

#[test]
fn a_row_shares_free_space_by_grow_after_fixed_sizes_padding_and_gaps() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let bar_style = Style::row()
        .padding(10.0)
        .gap(5.0)
        .height(Length::Px(100.0));
    let bar = ui
        .add(ui.root(), Probe::new("bar", &log), bar_style)
        .unwrap();
    let fixed = Style::row().width(Length::Px(50.0));
    let fixed = ui.add(bar, Probe::new("fixed", &log), fixed).unwrap();
    let one = ui
        .add(bar, Probe::new("one", &log), Style::row().grow(1.0))
        .unwrap();
    let three = ui
        .add(bar, Probe::new("three", &log), Style::row().grow(3.0))
        .unwrap();
    ui.layout();
    // 380 inside the padding, less 50 fixed and two 5 gaps: 320 shared 1:3.
    assert_eq!(ui.rect(bar), Some(Rect::new(0.0, 0.0, 400.0, 100.0)));
    assert_eq!(ui.rect(fixed), Some(Rect::new(10.0, 10.0, 50.0, 80.0)));
    assert_eq!(ui.rect(one), Some(Rect::new(65.0, 10.0, 80.0, 80.0)));
    assert_eq!(ui.rect(three), Some(Rect::new(150.0, 10.0, 240.0, 80.0)));
}

#[test]
fn leaves_are_measured_and_told_their_final_size() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let column = Style::column().align_items(Align::Start);
    let column = ui
        .add(ui.root(), Probe::new("column", &log), column)
        .unwrap();
    let label = ui
        .add(
            column,
            Probe::new("label", &log).sized(60.0, 24.0),
            Style::row(),
        )
        .unwrap();
    ui.layout();
    assert_eq!(ui.rect(label), Some(Rect::new(0.0, 0.0, 60.0, 24.0)));
    let probe = ui.widget::<Probe>(label).unwrap();
    assert_eq!(probe.laid_out, Some(Size::new(60.0, 24.0)));
    assert!(probe.measured > 0);
}

#[test]
fn lengths_are_logical_pixels_times_the_scale() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let style = Style::row()
        .size(50.0, 20.0)
        .margin(Edges::all(Length::Px(10.0)));
    let id = ui.add(ui.root(), Probe::new("box", &log), style).unwrap();
    ui.set_scale(2.0);
    ui.layout();
    assert_eq!(ui.rect(id), Some(Rect::new(20.0, 20.0, 100.0, 40.0)));
    assert_eq!(ui.scale(), 2.0);
}

#[test]
fn fractions_are_shares_of_the_parent_and_the_viewport_drives_the_root() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let style = Style::row().width(Length::Fraction(0.25)).grow(0.0);
    let quarter = ui
        .add(ui.root(), Probe::new("quarter", &log), style)
        .unwrap();
    let rest = ui
        .add(ui.root(), Probe::new("rest", &log), Style::row().grow(1.0))
        .unwrap();
    ui.layout();
    assert_eq!(ui.rect(quarter).unwrap().width(), 100.0);
    assert_eq!(ui.rect(rest).unwrap().height(), 300.0);
    ui.resize(Size::new(800.0, 500.0));
    ui.layout();
    assert_eq!(ui.rect(ui.root()), Some(Rect::new(0.0, 0.0, 800.0, 500.0)));
    assert_eq!(ui.rect(quarter).unwrap().width(), 200.0);
    assert_eq!(ui.rect(rest).unwrap().height(), 500.0);
}

#[test]
fn absolute_widgets_sit_at_their_inset_over_the_flow() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let panel = Style::column().padding(8.0).grow(1.0);
    let panel = ui.add(ui.root(), Probe::new("panel", &log), panel).unwrap();
    let inset = Edges {
        left: Length::Px(10.0),
        top: Length::Px(20.0),
        right: Length::Auto,
        bottom: Length::Auto,
    };
    let style = Style::row().absolute(inset).size(30.0, 30.0);
    let badge = ui.add(panel, Probe::new("badge", &log), style).unwrap();
    ui.layout();
    assert_eq!(ui.rect(badge), Some(Rect::new(10.0, 20.0, 30.0, 30.0)));
}

#[test]
fn hidden_widgets_take_no_space() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let bar = ui
        .add(ui.root(), Probe::new("bar", &log), Style::row())
        .unwrap();
    let a = ui
        .add(bar, Probe::new("a", &log), Style::row().size(50.0, 10.0))
        .unwrap();
    let b = ui
        .add(bar, Probe::new("b", &log), Style::row().size(50.0, 10.0))
        .unwrap();
    ui.set_style(a, Style::row().size(50.0, 10.0).hidden(true));
    ui.layout();
    assert_eq!(ui.rect(b).unwrap().left(), 0.0);
    assert!(ui.rect(a).unwrap().is_empty());
}

#[test]
fn layout_runs_only_when_something_changed() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let column = Style::column().align_items(Align::Start);
    let column = ui
        .add(ui.root(), Probe::new("column", &log), column)
        .unwrap();
    let leaf = Probe::new("leaf", &log).sized(10.0, 10.0);
    let leaf = ui.add(column, leaf, Style::row()).unwrap();
    ui.layout();
    let measured = |ui: &densa_ui::Ui| ui.widget::<Probe>(leaf).unwrap().measured;
    let first = measured(&ui);
    ui.layout();
    assert_eq!(measured(&ui), first, "nothing changed: not measured again");
    ui.update::<Probe, _>(leaf, |probe, cx| {
        probe.intrinsic = Size::new(40.0, 10.0);
        cx.request_layout();
    });
    assert!(ui.needs_paint());
    ui.layout();
    assert!(measured(&ui) > first);
    assert_eq!(ui.rect(leaf).unwrap().width(), 40.0);
}

#[test]
fn free_space_is_justified_and_items_aligned_one_by_one() {
    use densa_ui::Justify;
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let style = |justify| Style::row().justify(justify).height(Length::Px(50.0));
    let center = ui
        .add(
            ui.root(),
            Probe::new("center", &log),
            style(Justify::Center),
        )
        .unwrap();
    let a = ui
        .add(
            center,
            Probe::new("a", &log),
            Style::row().size(100.0, 20.0),
        )
        .unwrap();
    let spread = ui
        .add(
            ui.root(),
            Probe::new("spread", &log),
            style(Justify::SpaceBetween),
        )
        .unwrap();
    let b = ui
        .add(
            spread,
            Probe::new("b", &log),
            Style::row().size(100.0, 20.0),
        )
        .unwrap();
    let c = Style::row().size(100.0, 20.0).align_self(Align::End);
    let c = ui.add(spread, Probe::new("c", &log), c).unwrap();
    ui.layout();
    assert_eq!(ui.rect(a), Some(Rect::new(150.0, 0.0, 100.0, 20.0)));
    assert_eq!(ui.rect(b).unwrap().left(), 0.0);
    assert_eq!(ui.rect(c), Some(Rect::new(300.0, 80.0, 100.0, 20.0)));
}
