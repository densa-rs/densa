/// A position in pixels, y down.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// Horizontal position, rightwards.
    pub x: f32,
    /// Vertical position, downwards.
    pub y: f32,
}

impl Point {
    /// The origin, `(0, 0)`.
    pub const ORIGIN: Self = Self::new(0.0, 0.0);

    /// The point `(x, y)`.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// The point moved by `dx`, `dy`.
    #[must_use]
    pub fn offset(self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }

    /// `[x, y]`.
    pub const fn to_array(self) -> [f32; 2] {
        [self.x, self.y]
    }
}

impl From<[f32; 2]> for Point {
    fn from([x, y]: [f32; 2]) -> Self {
        Self::new(x, y)
    }
}

/// An axis-aligned rectangle in pixels, y down, stored as its four edges.
///
/// Edges, not origin plus size, so rectangles built from shared edges tile exactly: with
/// half-open [`contains`](Self::contains), a point on a shared edge belongs to exactly one of
/// them, even after both are [translated](Self::translate) by the same amount. (Origin plus
/// size recomputes `x + width`, which can miss the original edge by a rounding step.)
///
/// A rectangle whose right edge is not past its left (or bottom past top), or with a NaN
/// edge, is [empty](Self::is_empty) and contains no point.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Left edge.
    pub x0: f32,
    /// Top edge.
    pub y0: f32,
    /// Right edge.
    pub x1: f32,
    /// Bottom edge.
    pub y1: f32,
}

impl Rect {
    /// The rectangle with top-left corner `(x, y)` and size `width` x `height`.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self::from_edges(x, y, x + width, y + height)
    }

    /// The rectangle spanning the given edges, in either order. NaN edges are kept, giving
    /// an empty rectangle rather than a silently different one.
    pub fn from_edges(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        let (x0, x1) = ordered(left, right);
        let (y0, y1) = ordered(top, bottom);
        Self { x0, y0, x1, y1 }
    }

    /// Left edge.
    pub fn left(&self) -> f32 {
        self.x0
    }

    /// Top edge.
    pub fn top(&self) -> f32 {
        self.y0
    }

    /// Right edge.
    pub fn right(&self) -> f32 {
        self.x1
    }

    /// Bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y1
    }

    /// Distance between the left and right edges.
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    /// Distance between the top and bottom edges.
    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    /// Whether the rectangle covers no area (also true with a NaN edge).
    pub fn is_empty(&self) -> bool {
        !(self.x1 > self.x0 && self.y1 > self.y0)
    }

    /// Whether `point` lies inside: left and top edges included, right and bottom not, so a
    /// point on an edge shared by two rectangles belongs to exactly one.
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x0 && point.x < self.x1 && point.y >= self.y0 && point.y < self.y1
    }

    /// The rectangle moved by `by`.
    #[must_use]
    pub fn translate(self, by: Point) -> Self {
        Self {
            x0: self.x0 + by.x,
            y0: self.y0 + by.y,
            x1: self.x1 + by.x,
            y1: self.y1 + by.y,
        }
    }

    /// `[x, y, width, height]`, the layout GPU instances use.
    pub fn to_xywh(self) -> [f32; 4] {
        [self.x0, self.y0, self.width(), self.height()]
    }

    /// The top-left corner.
    pub fn origin(&self) -> Point {
        Point::new(self.x0, self.y0)
    }

    /// Width and height.
    pub fn size(&self) -> Size {
        Size::new(self.width(), self.height())
    }

    /// The area both rectangles cover; empty if they do not overlap or either is empty.
    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        if self.is_empty() || other.is_empty() {
            return Self::default();
        }
        Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }

    /// Every point: what a clip that clips nothing is.
    pub const EVERYTHING: Self = Self {
        x0: f32::NEG_INFINITY,
        y0: f32::NEG_INFINITY,
        x1: f32::INFINITY,
        y1: f32::INFINITY,
    };
}

/// A width and a height in pixels.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    /// Horizontal extent.
    pub width: f32,
    /// Vertical extent.
    pub height: f32,
}

impl Size {
    /// No extent.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    /// `width` x `height`.
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// `(a, b)` in ascending order; a NaN keeps its place instead of being dropped.
fn ordered(a: f32, b: f32) -> (f32, f32) {
    if b < a {
        (b, a)
    } else {
        (a, b)
    }
}
