//! Saving a dock's arrangement, as a value and as text, and restoring it by panel keys.

use densa_core::Size;
use densa_paragraph::TextSystem;
use densa_ui::{Ui, UiError, WidgetId};
use densa_widgets::{
    DockArea, DockAt, DockAxis, DockLayout, DockLayoutError, DockPanel, DockSide, Panel,
    MAX_LAYOUT_DEPTH,
};

fn empty_dock() -> (Ui, WidgetId) {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(800.0, 600.0));
    let dock = ui
        .add(ui.root(), DockArea::new(), DockArea::style())
        .unwrap();
    (ui, dock)
}

fn keyed(ui: &mut Ui, dock: WidgetId, key: &str, at: DockAt) -> WidgetId {
    let panel = DockPanel::new(key.to_uppercase()).with_key(key);
    DockArea::add_panel(ui, dock, Panel::new(), panel, at).unwrap()
}

fn area(ui: &Ui, dock: WidgetId) -> &DockArea {
    ui.widget::<DockArea>(dock).unwrap()
}

fn tabs(panels: &[&str], shown: usize) -> DockLayout {
    let panels = panels.iter().map(|k| (*k).to_owned()).collect();
    DockLayout::Tabs { panels, shown }
}

fn split(axis: DockAxis, ratio: f32, first: DockLayout, second: DockLayout) -> DockLayout {
    DockLayout::Split {
        axis,
        ratio,
        first: Box::new(first),
        second: Box::new(second),
    }
}

/// Orders and Trades (shown) on the left; Chart above Log on the right.
fn workspace() -> (Ui, WidgetId, [WidgetId; 4]) {
    let (mut ui, dock) = empty_dock();
    let orders = keyed(&mut ui, dock, "orders", DockAt::First);
    let trades = keyed(&mut ui, dock, "trades", DockAt::With(orders));
    let chart = keyed(
        &mut ui,
        dock,
        "chart",
        DockAt::Beside(orders, DockSide::Right),
    );
    let log = keyed(
        &mut ui,
        dock,
        "log",
        DockAt::Beside(chart, DockSide::Bottom),
    );
    ui.layout();
    (ui, dock, [orders, trades, chart, log])
}

const WORKSPACE: &str = r#"densa-dock 1 (row 0.5 (tabs 1 "orders" "trades") (column 0.5 (tabs 0 "chart") (tabs 0 "log")))"#;

/// The panels as keys, in their groups, with the shown ones in capitals.
fn groups(ui: &Ui, dock: WidgetId, panels: &[WidgetId]) -> Vec<Vec<String>> {
    let area = area(ui, dock);
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for &panel in panels {
        if seen.contains(&panel) {
            continue;
        }
        let group: Vec<WidgetId> = area.tabs_with(panel).collect();
        seen.extend(&group);
        out.push(
            group
                .iter()
                .map(|&p| {
                    let key = area.key_of(p).unwrap_or("-").to_owned();
                    if area.is_shown(p) {
                        key.to_uppercase()
                    } else {
                        key
                    }
                })
                .collect(),
        );
    }
    out
}

#[test]
fn a_layout_is_saved_as_a_line_of_text_and_read_back() {
    let (ui, dock, _) = workspace();
    let layout = area(&ui, dock).layout().unwrap();
    assert_eq!(layout.to_string(), WORKSPACE);
    assert_eq!(WORKSPACE.parse::<DockLayout>().unwrap(), layout);
}

#[test]
fn restoring_puts_every_panel_back_by_its_key() {
    let (ui, dock, _) = workspace();
    let saved = area(&ui, dock).layout().unwrap();
    // The same panels, added another way: all in one group.
    let (mut ui, dock) = empty_dock();
    let panels =
        ["log", "chart", "trades", "orders"].map(|k| keyed(&mut ui, dock, k, DockAt::First));
    DockArea::restore_layout(&mut ui, dock, &saved).unwrap();
    ui.layout();
    assert_eq!(area(&ui, dock).layout().unwrap(), saved);
    assert_eq!(area(&ui, dock).group_count(), 3);
    let [log, chart, trades, orders] = panels;
    assert_eq!(
        groups(&ui, dock, &[orders, chart, log]),
        [vec!["orders", "TRADES"], vec!["CHART"], vec!["LOG"]]
    );
    // The panels are where the groups are: Orders' group left of Chart's, Chart above Log.
    let r = |id| ui.rect(id).unwrap();
    assert!(r(trades).right() < r(chart).left());
    assert!(r(chart).bottom() < r(log).top());
}

#[test]
fn ratios_are_restored_and_kept_within_bounds() {
    let (mut ui, dock, [orders, _, chart, _]) = workspace();
    let wide = split(
        DockAxis::Row,
        0.75,
        tabs(&["orders"], 0),
        tabs(&["chart"], 0),
    );
    DockArea::restore_layout(&mut ui, dock, &wide).unwrap();
    ui.layout();
    let (left, right) = (ui.rect(orders).unwrap(), ui.rect(chart).unwrap());
    assert!(left.width() > 2.5 * right.width(), "{left:?} {right:?}");
    for (ratio, kept) in [(f32::NAN, 0.5), (2.0, 1.0), (-1.0, 0.0)] {
        let odd = split(
            DockAxis::Column,
            ratio,
            tabs(&["orders"], 0),
            tabs(&["chart"], 0),
        );
        DockArea::restore_layout(&mut ui, dock, &odd).unwrap();
        match area(&ui, dock).layout().unwrap() {
            DockLayout::Split { ratio, axis, .. } => {
                assert_eq!((ratio, axis), (kept, DockAxis::Column));
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn keys_without_panels_are_passed_over_and_their_groups_go() {
    let (mut ui, dock, [orders, trades, chart, log]) = workspace();
    let layout = split(
        DockAxis::Row,
        0.3,
        tabs(&["gone"], 0),
        split(
            DockAxis::Column,
            0.5,
            tabs(&["chart", "missing", "orders"], 1),
            tabs(&["log", "trades"], 0),
        ),
    );
    DockArea::restore_layout(&mut ui, dock, &layout).unwrap();
    assert_eq!(
        groups(&ui, dock, &[chart, log]),
        [vec!["CHART", "orders"], vec!["LOG", "trades"]],
        "the shown key was missing: the group shows its first tab"
    );
    let _ = (orders, trades);
    let expected = split(
        DockAxis::Column,
        0.5,
        tabs(&["chart", "orders"], 0),
        tabs(&["log", "trades"], 0),
    );
    assert_eq!(area(&ui, dock).layout().unwrap(), expected);
}

#[test]
fn panels_the_layout_does_not_name_join_the_first_group() {
    let (mut ui, dock, [orders, trades, chart, log]) = workspace();
    let scratch =
        DockArea::add_panel(&mut ui, dock, Panel::new(), "Scratch", DockAt::First).unwrap();
    let layout = split(DockAxis::Row, 0.5, tabs(&["chart"], 0), tabs(&["log"], 0));
    DockArea::restore_layout(&mut ui, dock, &layout).unwrap();
    assert_eq!(
        groups(&ui, dock, &[chart, log]),
        [vec!["CHART", "orders", "trades", "-"], vec!["LOG"]],
        "added after, keeping the shown tab"
    );
    let _ = (orders, trades, scratch);
    // Saving leaves out the panel without a key.
    let saved = area(&ui, dock).layout().unwrap();
    assert_eq!(
        saved,
        split(
            DockAxis::Row,
            0.5,
            tabs(&["chart", "orders", "trades"], 0),
            tabs(&["log"], 0)
        )
    );
}

#[test]
fn a_layout_naming_no_panel_leaves_one_group_showing_the_first() {
    let (mut ui, dock, [orders, trades, chart, log]) = workspace();
    DockArea::restore_layout(&mut ui, dock, &tabs(&["nothing"], 0)).unwrap();
    assert_eq!(area(&ui, dock).group_count(), 1);
    assert_eq!(
        groups(&ui, dock, &[orders]),
        [vec!["ORDERS", "trades", "chart", "log"]]
    );
    let _ = (trades, chart, log);
}

#[test]
fn panels_sharing_a_key_take_its_places_in_turn() {
    let (mut ui, dock) = empty_dock();
    let a = keyed(&mut ui, dock, "chart", DockAt::First);
    let b = keyed(&mut ui, dock, "chart", DockAt::With(a));
    let layout = split(DockAxis::Row, 0.5, tabs(&["chart"], 0), tabs(&["chart"], 0));
    DockArea::restore_layout(&mut ui, dock, &layout).unwrap();
    assert_eq!(area(&ui, dock).group_count(), 2);
    assert_eq!(area(&ui, dock).layout().unwrap(), layout);
    assert!(area(&ui, dock).is_shown(a) && area(&ui, dock).is_shown(b));
}

#[test]
fn a_dock_without_keys_has_no_layout_to_save() {
    let (mut ui, dock) = empty_dock();
    assert_eq!(area(&ui, dock).layout(), None);
    let plain = DockArea::add_panel(&mut ui, dock, Panel::new(), "Plain", DockAt::First).unwrap();
    assert_eq!(area(&ui, dock).layout(), None);
    // A keyed panel beside it: the unkeyed group's split collapses.
    keyed(
        &mut ui,
        dock,
        "chart",
        DockAt::Beside(plain, DockSide::Right),
    );
    assert_eq!(area(&ui, dock).layout(), Some(tabs(&["chart"], 0)));
}

#[test]
fn restoring_onto_a_widget_that_is_not_a_dock_fails() {
    let (mut ui, _) = empty_dock();
    let panel = ui.add(ui.root(), Panel::new(), Default::default()).unwrap();
    let layout = tabs(&["x"], 0);
    assert_eq!(
        DockArea::restore_layout(&mut ui, panel, &layout),
        Err(UiError::NoSuchWidget(panel))
    );
}

#[test]
fn keys_with_quotes_backslashes_and_any_text_read_back_the_same() {
    let keys = [
        "say \"hi\"",
        "C:\\path\\",
        "ünïcødé 订单",
        "",
        " spaced ",
        "a\nb",
        ")(",
    ];
    let layout = DockLayout::Tabs {
        panels: keys.iter().map(|k| (*k).to_owned()).collect(),
        shown: 3,
    };
    let text = layout.to_string();
    assert_eq!(text.parse::<DockLayout>().unwrap(), layout, "{text}");
}

#[test]
fn every_ratio_reads_back_exactly() {
    for ratio in [
        0.0,
        1.0,
        0.1,
        1.0 / 3.0,
        0.123_456_79,
        f32::MIN_POSITIVE,
        1e-30,
    ] {
        let layout = split(DockAxis::Row, ratio, tabs(&["a"], 0), tabs(&["b"], 0));
        assert_eq!(layout.to_string().parse::<DockLayout>().unwrap(), layout);
    }
}

fn error(text: &str) -> DockLayoutError {
    text.parse::<DockLayout>().unwrap_err()
}

#[test]
fn malformed_layouts_say_where_and_why() {
    let cases: [(&str, usize, &str); 17] = [
        ("", 0, "not a densa dock layout (version 1)"),
        (
            "densa-dock 2 (tabs 0 \"a\")",
            0,
            "not a densa dock layout (version 1)",
        ),
        (
            "densa-dock 1(tabs 0 \"a\")",
            12,
            "not a densa dock layout (version 1)",
        ),
        ("densa-dock 1", 12, "not a densa dock layout (version 1)"),
        ("densa-dock 1 ", 13, "expected `(`"),
        ("densa-dock 1 tabs", 13, "expected `(`"),
        (
            "densa-dock 1 (grid 0)",
            14,
            "expected `row`, `column` or `tabs`",
        ),
        (
            "densa-dock 1 (row 1.5 (tabs 0 \"a\") (tabs 0 \"b\"))",
            18,
            "expected a ratio from 0 to 1",
        ),
        (
            "densa-dock 1 (row NaN (tabs 0 \"a\") (tabs 0 \"b\"))",
            18,
            "expected a ratio from 0 to 1",
        ),
        ("densa-dock 1 (row 0.5 (tabs 0 \"a\"))", 34, "expected `(`"),
        (
            "densa-dock 1 (tabs x \"a\")",
            19,
            "expected the shown tab's position",
        ),
        (
            "densa-dock 1 (tabs 1 \"a\")",
            19,
            "a group must show one of its panels",
        ),
        (
            "densa-dock 1 (tabs 0)",
            19,
            "a group must show one of its panels",
        ),
        (
            "densa-dock 1 (tabs 0 \"a)",
            24,
            "a key without its closing quote",
        ),
        (
            "densa-dock 1 (tabs 0 \"a\\n\")",
            23,
            "only `\\\"` and `\\\\` are escapes",
        ),
        ("densa-dock 1 (tabs 0 \"a\" 7)", 25, "expected `)`"),
        ("densa-dock 1 (tabs 0 \"a\") x", 26, "text after the layout"),
    ];
    for (text, at, reason) in cases {
        let error = error(text);
        assert_eq!((error.at, error.reason), (at, reason), "{text:?}");
    }
    assert_eq!(
        error("densa-dock 1 ").to_string(),
        "invalid dock layout at byte 13: expected `(`"
    );
    // Space around the layout is fine.
    assert!(format!("  {WORKSPACE}\n").parse::<DockLayout>().is_ok());
}

/// `depth` splits, each holding a group and the next split.
fn nested(depth: usize) -> String {
    let mut text = String::from("densa-dock 1 ");
    for _ in 0..depth {
        text.push_str("(row 0.5 (tabs 0 \"a\") ");
    }
    text.push_str("(tabs 0 \"b\")");
    text.push_str(&")".repeat(depth));
    text
}

#[test]
fn splits_nest_only_so_deep() {
    assert!(nested(MAX_LAYOUT_DEPTH).parse::<DockLayout>().is_ok());
    let deep = error(&nested(MAX_LAYOUT_DEPTH + 1));
    assert_eq!(deep.reason, "splits nested too deeply");
    // Built in code instead: the levels past the limit are passed over.
    let (mut ui, dock) = empty_dock();
    let a = keyed(&mut ui, dock, "a", DockAt::First);
    let mut layout = tabs(&["a"], 0);
    for _ in 0..1_000 {
        layout = split(DockAxis::Row, 0.5, tabs(&["none"], 0), layout);
    }
    DockArea::restore_layout(&mut ui, dock, &layout).unwrap();
    assert!(
        area(&ui, dock).is_shown(a),
        "placed with the panels left over"
    );
}

#[test]
fn no_text_makes_reading_panic() {
    // Cuts, splices and stray characters of a real layout, chosen by a fixed sequence.
    let base: Vec<char> = WORKSPACE.chars().collect();
    let extra = ['(', ')', '"', '\\', ' ', '0', '9', '.', '-', 'e', 'é', '\n'];
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    for _ in 0..20_000 {
        let mut text = base.clone();
        for _ in 0..1 + next(4) {
            let at = next(text.len() + 1);
            match next(3) {
                0 if at < text.len() => {
                    text.remove(at);
                }
                1 => text.insert(at, extra[next(extra.len())]),
                _ => text.truncate(at),
            }
        }
        let text: String = text.into_iter().collect();
        if let Ok(layout) = text.parse::<DockLayout>() {
            assert_eq!(layout.to_string().parse::<DockLayout>().unwrap(), layout);
        }
    }
}
