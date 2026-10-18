//! A docking workspace: panels in tab groups that you rearrange by dragging their tabs onto
//! other groups or near their edges, and resize by dragging the gaps.
//!
//! `cargo run --release -p densa-widgets --example dock` (`--light` for the light theme,
//! `--seconds N` to close after N seconds, `--demo` to start with a tab being dragged).

use std::error::Error;
use std::time::{Duration, Instant};

use densa_core::Point;
use densa_ui::{
    Action, ActionKind, Align, PointerButton, PointerEvent, PointerKind, Style, Theme, Ui, WidgetId,
};
use densa_widgets::{DockArea, DockAt, DockPanel, DockSide, Label, Panel, ScrollView};
use densa_window::{App, Host, WindowOptions};

/// Instruments in the watchlist.
const NAMES: [&str; 12] = [
    "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "AVGO", "ASML", "AMD", "INTC", "ORCL",
];

struct Workspace {
    light: bool,
    demo: bool,
    close_at: Option<Instant>,
    dock: Option<WidgetId>,
    status: Option<WidgetId>,
}

/// A panel holding a scrolling list of `rows`, keyed by its title in lower case.
fn list_panel(
    ui: &mut Ui,
    dock: WidgetId,
    title: &str,
    at: DockAt,
    rows: impl Iterator<Item = String>,
) -> Result<WidgetId, Box<dyn Error>> {
    let spec = DockPanel::new(title).with_key(title.to_lowercase());
    let panel = DockArea::add_panel(ui, dock, Panel::new(), spec, at)?;
    let list = ui.add(panel, ScrollView::new(), ScrollView::style().padding(6.0))?;
    for row in rows {
        ui.add(list, Label::new(row), Style::row().padding(3.0).shrink(0.0))?;
    }
    Ok(panel)
}

/// A panel holding a heading and a wrapped paragraph.
fn text_panel(
    ui: &mut Ui,
    dock: WidgetId,
    title: &str,
    at: DockAt,
    text: &str,
) -> Result<WidgetId, Box<dyn Error>> {
    let panel = DockArea::add_panel(ui, dock, Panel::new(), title, at)?;
    let body = Style::column().grow(1.0).padding(14.0).gap(8.0);
    let body = ui.add(panel, Panel::transparent(), body)?;
    ui.add(body, Label::new(title).with_size(18.0), Style::row())?;
    ui.add(body, Label::new(text).wrapping().dim(), Style::row())?;
    Ok(panel)
}

fn quote(index: usize) -> String {
    let name = NAMES[index % NAMES.len()];
    let price = 40.0 + (index * 37 % 500) as f64 + (index * 13 % 100) as f64 / 100.0;
    let change = (index * 29 % 400) as f64 / 100.0 - 2.0;
    format!("{name:<7}{price:>9.2}{change:>+7.2}%")
}

fn log_line(index: usize) -> String {
    let (h, m, s) = (9 + index / 60 % 8, index % 60, index * 7 % 60);
    format!("{h:02}:{m:02}:{s:02}  event {index:>4}  ok")
}

impl App for Workspace {
    fn build(&mut self, ui: &mut Ui, _: &mut Host<'_>) -> Result<(), Box<dyn Error>> {
        if self.light {
            ui.set_theme(Theme::light());
        }
        let root = ui.root();
        let dock = ui.add(root, DockArea::new(), DockArea::style().padding(0.0))?;
        let watch = list_panel(ui, dock, "Watchlist", DockAt::First, (0..60).map(quote))?;
        let about = "Drag a tab along its bar to reorder, onto another group to join it, or \
                     near a group's edge to split the group. Drag the gaps between groups to \
                     resize them. Painting and dragging tabs allocate nothing.";
        let details = text_panel(
            ui,
            dock,
            "Details",
            DockAt::Beside(watch, DockSide::Right),
            about,
        )?;
        let below = DockAt::Beside(details, DockSide::Bottom);
        let log = list_panel(ui, dock, "Log", below, (0..200).map(log_line))?;
        text_panel(ui, dock, "Notes", DockAt::With(log), "Tabs share a group.")?;
        list_panel(
            ui,
            dock,
            "Screener",
            DockAt::With(watch),
            (12..40).map(quote),
        )?;
        ui.update::<DockArea, _>(dock, |area, cx| area.show(cx, watch));

        let bar = Style::row()
            .padding(6.0)
            .shrink(0.0)
            .align_items(Align::Center);
        let bar = ui.add(root, Panel::new(), bar)?;
        self.status = Some(ui.add(bar, Label::new("Ready").dim(), Style::row())?);
        self.dock = Some(dock);
        Ok(())
    }

    fn action(&mut self, ui: &mut Ui, _: &mut Host<'_>, action: Action) {
        let (Some(dock), Some(status)) = (self.dock, self.status) else {
            return;
        };
        if action.source != dock {
            return;
        }
        let groups = ui.widget::<DockArea>(dock).map_or(0, DockArea::group_count);
        let Some(area) = ui.widget::<DockArea>(dock) else {
            return;
        };
        let text = match action.kind {
            ActionKind::Selected(index) => {
                let title = area.panels().nth(index).and_then(|p| area.title(p));
                format!("Showing {}", title.unwrap_or("?"))
            }
            ActionKind::Changed => format!("Rearranged: {groups} groups"),
            _ => return,
        };
        ui.update::<Label, _>(status, |label, cx| label.set_text(cx, text));
    }

    fn frame(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> bool {
        if std::mem::take(&mut self.demo) {
            // The Screener tab held over the right of the Log group, once the window has its
            // size (it has none yet in `build`).
            if let Some((from, to)) = self.dock.and_then(|dock| demo_drag(ui, dock)) {
                let press = PointerKind::Down(PointerButton::Primary);
                ui.pointer(PointerEvent::new(press, from));
                ui.pointer(PointerEvent::new(PointerKind::Move, to));
            }
        }
        match self.close_at {
            Some(at) if Instant::now() >= at => {
                host.close();
                false
            }
            Some(_) => true,
            None => false,
        }
    }
}

/// Where `--demo` presses (the Screener tab) and where it holds the tab (near the right edge
/// of the group the Log panel is in).
fn demo_drag(ui: &mut Ui, dock: WidgetId) -> Option<(Point, Point)> {
    ui.layout();
    let area = ui.widget::<DockArea>(dock)?;
    let screener = area.panel_with_key("screener")?;
    let tab = area.tab_rect(screener, ui.rect(dock)?, ui.scale())?;
    let log = area.panel_with_key("log")?;
    let shown = area.tabs_with(log).find(|&p| area.is_shown(p))?;
    let group = ui.rect(shown)?;
    let from = Point::new(
        (tab.left() + tab.right()) / 2.0,
        (tab.top() + tab.bottom()) / 2.0,
    );
    let to = Point::new(
        group.right() - group.width() / 10.0,
        (group.top() + group.bottom()) / 2.0,
    );
    Some((from, to))
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let light = args.iter().any(|a| a == "--light");
    let demo = args.iter().any(|a| a == "--demo");
    let seconds = args
        .iter()
        .skip_while(|a| *a != "--seconds")
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let app = Workspace {
        light,
        demo,
        close_at: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        dock: None,
        status: None,
    };
    densa_window::run(
        WindowOptions::new("densa dock").with_size(1200.0, 760.0),
        app,
    )?;
    Ok(())
}
