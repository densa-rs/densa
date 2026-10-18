//! The standard widgets in a window: a toolbar, a filterable list beside a details pane in a
//! splitter, and a status bar.
//!
//! `cargo run --release -p densa-widgets --example gallery` (`--light` for the light theme,
//! `--seconds N` to close after N seconds).

use std::error::Error;
use std::time::Instant;

use densa_ui::{Action, ActionKind, Align, Length, Style, Theme, Ui, WidgetId};
use densa_widgets::{Button, Label, Panel, ScrollView, Splitter, TextInput};
use densa_window::{App, Host, WindowOptions};

/// Instruments in the list.
const NAMES: [&str; 12] = [
    "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "AVGO", "ASML", "AMD", "INTC", "ORCL",
];

struct Gallery {
    light: bool,
    close_at: Option<Instant>,
    ids: Option<Ids>,
    added: usize,
}

/// The widgets the gallery acts on.
#[derive(Clone, Copy)]
struct Ids {
    filter: WidgetId,
    add: WidgetId,
    clear: WidgetId,
    theme: WidgetId,
    run: WidgetId,
    list: WidgetId,
    status: WidgetId,
}

/// A list row: a name and a price, aligned in the monospaced font.
fn row(index: usize) -> String {
    let name = NAMES[index % NAMES.len()];
    let price = 40.0 + (index * 37 % 500) as f64 + (index * 13 % 100) as f64 / 100.0;
    format!("{name:<8}{:>4}  {price:>9.2}", index + 1)
}

impl Gallery {
    fn add_row(&mut self, ui: &mut Ui, list: WidgetId) -> Result<(), Box<dyn Error>> {
        let style = Style::row().padding(4.0).shrink(0.0);
        ui.add(list, Label::new(row(self.added)), style)?;
        self.added += 1;
        Ok(())
    }

    fn status(ui: &mut Ui, status: WidgetId, text: String) {
        ui.update::<Label, _>(status, |label, cx| label.set_text(cx, text));
    }

    /// Shows only the rows containing the filter text.
    fn filter(ui: &mut Ui, ids: &Ids) {
        let needle = ui
            .widget::<TextInput>(ids.filter)
            .map(|input| input.text().to_uppercase())
            .unwrap_or_default();
        let rows: Vec<WidgetId> = ui.children(ids.list).to_vec();
        let mut shown = 0;
        for id in rows {
            let matches = ui
                .widget::<Label>(id)
                .is_some_and(|label| label.text().contains(&needle));
            shown += usize::from(matches);
            let style = Style::row().padding(4.0).shrink(0.0).hidden(!matches);
            ui.set_style(id, style);
        }
        Self::status(ui, ids.status, format!("{shown} rows match \"{needle}\""));
    }
}

impl App for Gallery {
    fn build(&mut self, ui: &mut Ui, _: &mut Host<'_>) -> Result<(), Box<dyn Error>> {
        if self.light {
            ui.set_theme(Theme::light());
        }
        let root = ui.root();
        let toolbar = Style::row()
            .gap(6.0)
            .padding(8.0)
            .align_items(Align::Center);
        let toolbar = ui.add(root, Panel::new(), toolbar.shrink(0.0))?;
        ui.add(
            toolbar,
            Label::new("densa widgets").with_size(15.0),
            Style::row(),
        )?;
        let spacer = Style::row().grow(1.0);
        ui.add(toolbar, Panel::transparent(), spacer)?;
        let filter = TextInput::new("").with_placeholder("Filter by name");
        let filter = ui.add(toolbar, filter, Style::row())?;
        let add = ui.add(toolbar, Button::new("Add row"), Style::row())?;
        let clear = ui.add(toolbar, Button::new("Clear"), Style::row())?;
        let theme = ui.add(toolbar, Button::new("Theme"), Style::row())?;
        let run = ui.add(toolbar, Button::new("Run").primary(), Style::row())?;

        let splitter = Splitter::row();
        let style = splitter.style().padding(6.0);
        let body = ui.add(root, splitter, style)?;
        let left = Style::column().width(Length::Px(300.0)).shrink(0.0);
        let left = ui.add(body, Panel::new().with_border(), left)?;
        let list = ui.add(left, ScrollView::new(), ScrollView::style().padding(4.0))?;
        let right = Style::column().grow(1.0).padding(16.0).gap(10.0);
        let right = ui.add(body, Panel::new().with_border(), right)?;
        ui.add(right, Label::new("Details").with_size(20.0), Style::row())?;
        let about = "Widgets lay out with flexbox and paint into one draw call per frame. \
                     Drag the divider, scroll the list, type a filter, press Tab to move \
                     between controls. Nothing allocates while painting or handling the \
                     pointer.";
        ui.add(right, Label::new(about).wrapping().dim(), Style::row())?;

        let status_bar = Style::row().padding(6.0).shrink(0.0);
        let status_bar = ui.add(root, Panel::new(), status_bar)?;
        let status = ui.add(status_bar, Label::new("Ready").dim(), Style::row())?;

        let ids = Ids {
            filter,
            add,
            clear,
            theme,
            run,
            list,
            status,
        };
        for _ in 0..40 {
            self.add_row(ui, ids.list)?;
        }
        ui.set_focus(Some(ids.filter));
        self.ids = Some(ids);
        Ok(())
    }

    fn action(&mut self, ui: &mut Ui, _: &mut Host<'_>, action: Action) {
        let Some(ids) = self.ids else {
            return;
        };
        let (source, kind) = (action.source, action.kind);
        match kind {
            ActionKind::Clicked if source == ids.add => {
                if self.add_row(ui, ids.list).is_ok() {
                    let rows = ui.children(ids.list).len();
                    Self::status(ui, ids.status, format!("{rows} rows"));
                }
            }
            ActionKind::Clicked if source == ids.clear => {
                for id in ui.children(ids.list).to_vec() {
                    ui.remove(id);
                }
                Self::status(ui, ids.status, "Cleared".to_owned());
            }
            ActionKind::Clicked if source == ids.theme => {
                self.light = !self.light;
                let theme = if self.light {
                    Theme::light()
                } else {
                    Theme::dark()
                };
                ui.set_theme(theme);
            }
            ActionKind::Clicked | ActionKind::Submitted
                if source == ids.run || source == ids.filter =>
            {
                Self::filter(ui, &ids);
            }
            ActionKind::Changed if source == ids.filter => Self::filter(ui, &ids),
            _ => {}
        }
    }

    fn frame(&mut self, _: &mut Ui, host: &mut Host<'_>) -> bool {
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

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let light = args.iter().any(|a| a == "--light");
    let seconds = args
        .iter()
        .skip_while(|a| *a != "--seconds")
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let close_at = seconds.map(|s| Instant::now() + std::time::Duration::from_secs(s));
    let app = Gallery {
        light,
        close_at,
        ids: None,
        added: 0,
    };
    densa_window::run(
        WindowOptions::new("densa widgets").with_size(1100.0, 700.0),
        app,
    )?;
    Ok(())
}
