//! Standard widgets for `densa_ui`: [`Label`], [`Button`], [`TextInput`], [`ScrollView`],
//! [`Splitter`], [`Panel`], [`DockArea`] (panels in tab groups the user rearranges),
//! [`DataGrid`] (rows of a model, scrolled, selected and sorted) and [`Chart`] (time series,
//! candles and volume, panned and zoomed).
//!
//! Each is a [`Widget`](densa_ui::Widget): add it to a [`Ui`](densa_ui::Ui) with a
//! [`Style`](densa_ui::Style), and read what the user did from the actions it reports.
//! They take their colors and text size from the [`Theme`](densa_ui::Theme), and paint and
//! handle input without allocating (editing text re-lays it out, which does).
//!
//! ```
//! use densa_core::Size;
//! use densa_paragraph::TextSystem;
//! use densa_ui::{Align, Style, Ui};
//! use densa_widgets::{Button, Label, Panel, TextInput};
//!
//! let mut ui = Ui::new(TextSystem::new()?);
//! ui.resize(Size::new(640.0, 480.0));
//! let bar = ui.add(
//!     ui.root(),
//!     Panel::new(),
//!     Style::row().gap(6.0).padding(8.0).align_items(Align::Center),
//! )?;
//! ui.add(bar, Label::new("Symbol"), Style::row())?;
//! let filter = ui.add(bar, TextInput::new("").with_placeholder("Filter"), Style::row())?;
//! let run = ui.add(bar, Button::new("Run").primary(), Style::row())?;
//! ui.layout();
//! assert!(ui.rect(run).unwrap().left() > ui.rect(filter).unwrap().right());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod button;
mod chart;
mod dock;
mod grid;
mod label;
mod paint;
mod panel;
mod scroll;
mod splitter;
mod text;
mod text_input;

pub use button::Button;
pub use chart::Chart;
pub use dock::{DockArea, DockAt, DockPanel, DockSide};
pub use grid::DataGrid;
pub use label::Label;
pub use panel::Panel;
pub use scroll::ScrollView;
pub use splitter::Splitter;
pub use text_input::TextInput;
