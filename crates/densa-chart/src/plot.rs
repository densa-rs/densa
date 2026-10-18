use densa_core::Rect;

use crate::Axis;

/// Where a chart draws and what it shows: an area of the canvas, with `x` running left to
/// right across it and `y` bottom to top.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plot {
    area: Rect,
    x: Axis,
    y: Axis,
}

impl Plot {
    /// A plot of `x` and `y` drawn in `area`.
    pub fn new(area: Rect, x: Axis, y: Axis) -> Self {
        Self { area, x, y }
    }

    /// The area drawn in.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// The horizontal axis.
    pub fn x(&self) -> Axis {
        self.x
    }

    /// The vertical axis.
    pub fn y(&self) -> Axis {
        self.y
    }

    /// The plot with another horizontal axis.
    #[must_use]
    pub fn with_x(self, x: Axis) -> Self {
        Self { x, ..self }
    }

    /// The plot with another vertical axis.
    #[must_use]
    pub fn with_y(self, y: Axis) -> Self {
        Self { y, ..self }
    }

    /// The canvas `x` of `value`.
    pub fn x_px(&self, value: f64) -> f32 {
        // Lossless enough: pixels.
        self.x_at(value) as f32
    }

    /// The canvas `y` of `value`.
    pub fn y_px(&self, value: f64) -> f32 {
        // Lossless enough: pixels.
        self.y_at(value) as f32
    }

    /// The value at canvas `x`.
    pub fn x_value(&self, px: f32) -> f64 {
        let width = f64::from(self.area.width());
        self.x
            .value_at((f64::from(px) - f64::from(self.area.x0)) / width)
    }

    /// The value at canvas `y`.
    pub fn y_value(&self, px: f32) -> f64 {
        let height = f64::from(self.area.height());
        self.y
            .value_at((f64::from(self.area.y1) - f64::from(px)) / height)
    }

    /// The canvas `x` of `value`, unrounded.
    pub(crate) fn x_at(&self, value: f64) -> f64 {
        f64::from(self.area.x0) + self.x.fraction(value) * f64::from(self.area.width())
    }

    /// The canvas `y` of `value`, unrounded.
    pub(crate) fn y_at(&self, value: f64) -> f64 {
        f64::from(self.area.y1) - self.y.fraction(value) * f64::from(self.area.height())
    }

    /// How many pixels `distance` along the horizontal axis takes.
    pub(crate) fn x_length(&self, distance: f64) -> f64 {
        distance / self.x.span() * f64::from(self.area.width())
    }

    /// `y` held within a pixel of the area, so far-off values stay finite and small.
    pub(crate) fn clamp_y(&self, y: f64) -> f64 {
        let (top, bottom) = (f64::from(self.area.y0), f64::from(self.area.y1));
        y.clamp(top - 1.0, bottom + 1.0)
    }
}
