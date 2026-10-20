//! Random sequences of adds, removals, tab drags and divider drags: after every step the dock
//! must still show one panel per group, its groups must hold every panel once, and the
//! panels it shows must not overlap.

use densa_core::{Point, Rect, Size};
use densa_paragraph::TextSystem;
use densa_ui::{PointerButton, PointerEvent, PointerKind, Ui, WidgetId};
use densa_widgets::{DockArea, DockAt, DockLayout, DockPanel, DockSide, Panel};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 700.0;

/// A small deterministic generator (wyrand), so a failure names the seed that reproduces it.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0xa076_1d64_78bd_642f);
        let t = u128::from(self.0) * u128::from(self.0 ^ 0xe703_7ed1_a0b4_28db);
        (t as u64) ^ ((t >> 64) as u64)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn coordinate(&mut self, max: f32) -> f32 {
        self.below(max as usize) as f32
    }
}

fn pointer(ui: &mut Ui, kind: PointerKind, x: f32, y: f32) {
    ui.pointer(PointerEvent::new(kind, Point::new(x, y)));
}

/// Where a step adds a panel: first, with or beside one of `panels`.
fn place(rng: &mut Rng, panels: &[WidgetId]) -> DockAt {
    if panels.is_empty() || rng.below(4) == 0 {
        return DockAt::First;
    }
    let p = panels[rng.below(panels.len())];
    match rng.below(5) {
        0 => DockAt::With(p),
        1 => DockAt::Beside(p, DockSide::Left),
        2 => DockAt::Beside(p, DockSide::Right),
        3 => DockAt::Beside(p, DockSide::Top),
        _ => DockAt::Beside(p, DockSide::Bottom),
    }
}

/// A press somewhere (often on a tab bar), a few moves, then a release or a cancel.
fn gesture(rng: &mut Rng, ui: &mut Ui) {
    let x = rng.coordinate(WIDTH);
    let y = if rng.below(2) == 0 {
        14.0
    } else {
        rng.coordinate(HEIGHT)
    };
    pointer(ui, PointerKind::Down(PointerButton::Primary), x, y);
    for _ in 0..1 + rng.below(3) {
        let (x, y) = (rng.coordinate(WIDTH), rng.coordinate(HEIGHT));
        pointer(ui, PointerKind::Move, x, y);
    }
    let (x, y) = (rng.coordinate(WIDTH), rng.coordinate(HEIGHT));
    if rng.below(5) == 0 {
        ui.cancel_pointer();
    } else {
        pointer(ui, PointerKind::Up(PointerButton::Primary), x, y);
    }
}

fn check(ui: &Ui, dock: WidgetId, context: &str) {
    let area = ui.widget::<DockArea>(dock).unwrap();
    let all: Vec<WidgetId> = area.panels().collect();
    let shown: Vec<WidgetId> = all.iter().copied().filter(|&p| area.is_shown(p)).collect();
    assert_eq!(
        shown.len(),
        area.group_count(),
        "{context}: one shown panel per group"
    );
    let mut groups: Vec<Vec<WidgetId>> = Vec::new();
    for &p in &all {
        let tabs: Vec<WidgetId> = area.tabs_with(p).collect();
        assert!(tabs.contains(&p), "{context}: a panel is in its own group");
        if !groups.contains(&tabs) {
            groups.push(tabs);
        }
    }
    let grouped: usize = groups.iter().map(Vec::len).sum();
    assert_eq!(
        grouped,
        all.len(),
        "{context}: the groups partition the panels"
    );
    let rects: Vec<Rect> = shown.iter().map(|&p| ui.rect(p).unwrap()).collect();
    for (i, a) in rects.iter().enumerate() {
        for b in &rects[i + 1..] {
            assert!(
                a.intersect(*b).is_empty(),
                "{context}: {a:?} overlaps {b:?}"
            );
        }
    }
}

/// Each panel's group (as its tabs) and whether it is shown.
fn arrangement(ui: &Ui, dock: WidgetId) -> Vec<(Vec<WidgetId>, bool)> {
    let area = ui.widget::<DockArea>(dock).unwrap();
    area.panels()
        .map(|p| (area.tabs_with(p).collect(), area.is_shown(p)))
        .collect()
}

#[test]
fn random_edits_keep_every_panel_in_exactly_one_group() {
    for seed in 1..120u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut ui = Ui::new(TextSystem::bundled().unwrap());
        ui.resize(Size::new(WIDTH, HEIGHT));
        let dock = ui
            .add(ui.root(), DockArea::new(), DockArea::style())
            .unwrap();
        for step in 0..100 {
            let panels: Vec<WidgetId> = ui.widget::<DockArea>(dock).unwrap().panels().collect();
            match rng.below(7) {
                0 | 1 => {
                    let at = place(&mut rng, &panels);
                    let panel = DockPanel::new(format!("T{step}")).with_key(format!("k{step}"));
                    DockArea::add_panel(&mut ui, dock, Panel::new(), panel, at).unwrap();
                }
                // Saving, writing, reading and restoring the layout changes nothing.
                3 => {
                    let before = arrangement(&ui, dock);
                    let area = ui.widget::<DockArea>(dock).unwrap();
                    if let Some(layout) = area.layout() {
                        let read: DockLayout = layout.to_string().parse().unwrap();
                        DockArea::restore_layout(&mut ui, dock, &read).unwrap();
                        let after = ui.widget::<DockArea>(dock).unwrap().layout();
                        assert_eq!(after.as_ref(), Some(&layout), "seed {seed} step {step}");
                        assert_eq!(arrangement(&ui, dock), before, "seed {seed} step {step}");
                    }
                }
                // Removals may empty the dock, which must then take panels again.
                2 if !panels.is_empty() => {
                    let gone = panels[rng.below(panels.len())];
                    DockArea::remove_panel(&mut ui, dock, gone).unwrap();
                }
                _ => {
                    ui.layout();
                    gesture(&mut rng, &mut ui);
                }
            }
            ui.layout();
            check(&ui, dock, &format!("seed {seed} step {step}"));
        }
    }
}
