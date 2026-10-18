use densa_core::Color;

/// Chart colors: [`ChartPalette::new`] is the dark default; change its fields for others.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ChartPalette {
    /// Behind the plot and its labels.
    pub background: Color,
    /// Gridlines at the ticks.
    pub grid: Color,
    /// The plot's bottom and right edges.
    pub axis: Color,
    /// Tick labels.
    pub label: Color,
    /// A line series without a color of its own.
    pub line: Color,
    /// Rising values: candles that closed up.
    pub up: Color,
    /// Falling values: candles that closed down.
    pub down: Color,
    /// The crosshair's lines.
    pub crosshair: Color,
    /// Behind the crosshair's values.
    pub marker: Color,
    /// The crosshair's values.
    pub marker_text: Color,
}

impl ChartPalette {
    /// The default dark palette.
    pub fn new() -> Self {
        Self {
            background: Color::srgb(0x0D1117),
            grid: Color::srgb(0x161B22),
            axis: Color::srgb(0x30363D),
            label: Color::srgb(0x8B949E),
            line: Color::srgb(0x58A6FF),
            up: Color::srgb(0x3FB950),
            down: Color::srgb(0xF85149),
            crosshair: Color::srgb(0x8B949E).with_alpha(0.6),
            marker: Color::srgb(0x484F58),
            marker_text: Color::srgb(0xF0F6FC),
        }
    }
}

impl Default for ChartPalette {
    fn default() -> Self {
        Self::new()
    }
}
