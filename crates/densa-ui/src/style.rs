//! How a widget is sized and how it places its children: flexbox, in logical pixels.

use taffy::style::{
    AlignContent, AlignItems, Dimension, Display, FlexDirection, FlexWrap, LengthPercentage,
    LengthPercentageAuto, Overflow as TaffyOverflow, Position as TaffyPosition,
};

/// A length along one axis.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[non_exhaustive]
pub enum Length {
    /// Decided by layout: the content's size, or what flexing gives.
    #[default]
    Auto,
    /// Logical pixels, multiplied by the UI's scale factor.
    Px(f32),
    /// A share of the parent's size: `0.5` is half.
    Fraction(f32),
}

/// One value per side of a box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges<T> {
    /// Left side.
    pub left: T,
    /// Top side.
    pub top: T,
    /// Right side.
    pub right: T,
    /// Bottom side.
    pub bottom: T,
}

impl<T: Copy> Edges<T> {
    /// `value` on every side.
    pub const fn all(value: T) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }

    /// `horizontal` on the left and right, `vertical` on the top and bottom.
    pub const fn symmetric(horizontal: T, vertical: T) -> Self {
        Self {
            left: horizontal,
            top: vertical,
            right: horizontal,
            bottom: vertical,
        }
    }
}

/// The axis children are laid out along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Direction {
    /// Left to right.
    #[default]
    Row,
    /// Top to bottom.
    Column,
}

/// Where items sit across the main axis (or one item, as [`Style::align_self`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Align {
    /// Filling the cross axis.
    #[default]
    Stretch,
    /// At the start.
    Start,
    /// Centered.
    Center,
    /// At the end.
    End,
}

/// How free space along the main axis is shared out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Justify {
    /// Items packed at the start.
    #[default]
    Start,
    /// Items centered.
    Center,
    /// Items packed at the end.
    End,
    /// Equal space between items, none at the ends.
    SpaceBetween,
    /// Equal space around every item.
    SpaceAround,
}

/// What happens to content larger than the widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Overflow {
    /// Shown outside the widget.
    #[default]
    Visible,
    /// Cut at the widget's edges.
    Clip,
    /// Cut at the edges and scrollable: its size does not grow the widget.
    Scroll,
}

/// How a widget takes part in its parent's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Position {
    /// In the flow of its siblings.
    #[default]
    InFlow,
    /// Placed by [`Style::inset`] against the parent, on top of its siblings.
    Absolute,
}

/// A widget's layout style: its size, spacing, and how it lays out its children (flexbox).
/// Lengths are in logical pixels.
///
/// ```
/// use densa_ui::{Align, Length, Style};
///
/// let toolbar = Style::row().gap(4.0).padding(6.0).align_items(Align::Center);
/// let content = Style::column().grow(1.0).height(Length::Auto);
/// # let _ = (toolbar, content);
/// ```
///
/// The default is [`Style::row`]: CSS's defaults (`flex-shrink: 1`, items stretched).
///
/// As in CSS, a widget is never laid out smaller than its content unless its minimum size
/// says so (`min-width: auto`): a region meant to shrink and scroll its content, and every
/// container between it and the window, needs `min_size(Length::Px(0.0), Length::Px(0.0))`
/// (or `Overflow::Clip` or `Overflow::Scroll`, which imply it for the widget itself).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Style {
    /// Not shown and taking no space.
    pub hidden: bool,
    /// The axis children are laid out along.
    pub direction: Direction,
    /// Whether children wrap onto more lines when they do not fit.
    pub wrap: bool,
    /// Width.
    pub width: Length,
    /// Height.
    pub height: Length,
    /// Smallest width.
    pub min_width: Length,
    /// Smallest height.
    pub min_height: Length,
    /// Largest width.
    pub max_width: Length,
    /// Largest height.
    pub max_height: Length,
    /// Share of free space along the parent's main axis.
    pub grow: f32,
    /// Share of missing space given up when the parent is too small.
    pub shrink: f32,
    /// Size along the parent's main axis before growing or shrinking.
    pub basis: Length,
    /// Space between the edges and the content.
    pub padding: Edges<f32>,
    /// Space outside the edges; `Auto` takes free space (pushing siblings away).
    pub margin: Edges<Length>,
    /// Space between children.
    pub gap: f32,
    /// Where children sit across the main axis.
    pub align_items: Align,
    /// Where this widget sits across its parent's main axis, instead of the parent's
    /// `align_items`.
    pub align_self: Option<Align>,
    /// How free space along the main axis is shared out.
    pub justify: Justify,
    /// In the flow of its siblings, or placed by `inset`.
    pub position: Position,
    /// Distances from the parent's edges, for [`Position::Absolute`].
    pub inset: Edges<Length>,
    /// What happens to content larger than the widget.
    pub overflow: Overflow,
    /// Width divided by height, kept when one of them is decided by layout.
    pub aspect_ratio: Option<f32>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            hidden: false,
            direction: Direction::Row,
            wrap: false,
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            min_height: Length::Auto,
            max_width: Length::Auto,
            max_height: Length::Auto,
            grow: 0.0,
            shrink: 1.0,
            basis: Length::Auto,
            padding: Edges::all(0.0),
            margin: Edges::all(Length::Px(0.0)),
            gap: 0.0,
            align_items: Align::Stretch,
            align_self: None,
            justify: Justify::Start,
            position: Position::InFlow,
            inset: Edges::all(Length::Auto),
            overflow: Overflow::Visible,
            aspect_ratio: None,
        }
    }
}

impl Style {
    /// Children left to right.
    pub fn row() -> Self {
        Self::default()
    }

    /// Children top to bottom.
    pub fn column() -> Self {
        Self {
            direction: Direction::Column,
            ..Self::row()
        }
    }

    /// The style with `width`.
    #[must_use]
    pub fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    /// The style with `height`.
    #[must_use]
    pub fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }

    /// The style with a fixed size in logical pixels.
    #[must_use]
    pub fn size(self, width: f32, height: f32) -> Self {
        self.width(Length::Px(width)).height(Length::Px(height))
    }

    /// The style with smallest sizes.
    #[must_use]
    pub fn min_size(mut self, width: Length, height: Length) -> Self {
        self.min_width = width;
        self.min_height = height;
        self
    }

    /// The style with largest sizes.
    #[must_use]
    pub fn max_size(mut self, width: Length, height: Length) -> Self {
        self.max_width = width;
        self.max_height = height;
        self
    }

    /// The style taking a `grow` share of free space.
    #[must_use]
    pub fn grow(mut self, grow: f32) -> Self {
        self.grow = grow;
        self
    }

    /// The style giving up a `shrink` share of missing space.
    #[must_use]
    pub fn shrink(mut self, shrink: f32) -> Self {
        self.shrink = shrink;
        self
    }

    /// The style with a size along the main axis before growing or shrinking.
    #[must_use]
    pub fn basis(mut self, basis: Length) -> Self {
        self.basis = basis;
        self
    }

    /// The style with `padding` on every side.
    #[must_use]
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = Edges::all(padding);
        self
    }

    /// The style with these paddings.
    #[must_use]
    pub fn padding_edges(mut self, padding: Edges<f32>) -> Self {
        self.padding = padding;
        self
    }

    /// The style with these margins.
    #[must_use]
    pub fn margin(mut self, margin: Edges<Length>) -> Self {
        self.margin = margin;
        self
    }

    /// The style with `gap` between children.
    #[must_use]
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// The style placing children across the main axis.
    #[must_use]
    pub fn align_items(mut self, align: Align) -> Self {
        self.align_items = align;
        self
    }

    /// The style placing this widget across its parent's main axis.
    #[must_use]
    pub fn align_self(mut self, align: Align) -> Self {
        self.align_self = Some(align);
        self
    }

    /// The style sharing out free space along the main axis.
    #[must_use]
    pub fn justify(mut self, justify: Justify) -> Self {
        self.justify = justify;
        self
    }

    /// The style placed at `inset` from the parent's edges, over its siblings.
    #[must_use]
    pub fn absolute(mut self, inset: Edges<Length>) -> Self {
        self.position = Position::Absolute;
        self.inset = inset;
        self
    }

    /// The style with `overflow`.
    #[must_use]
    pub fn overflow(mut self, overflow: Overflow) -> Self {
        self.overflow = overflow;
        self
    }

    /// The style wrapping children onto more lines, or not.
    #[must_use]
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// The style hidden (taking no space), or not.
    #[must_use]
    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// The style keeping width divided by height at `ratio`.
    #[must_use]
    pub fn aspect_ratio(mut self, ratio: f32) -> Self {
        self.aspect_ratio = Some(ratio);
        self
    }

    /// The layout engine's style, in physical pixels (`scale` per logical pixel).
    pub(crate) fn to_taffy(&self, scale: f32) -> taffy::Style {
        let px = |v: f32| v * scale;
        let dimension = |l: Length| match l {
            Length::Auto => Dimension::auto(),
            Length::Px(v) => Dimension::length(px(v)),
            Length::Fraction(f) => Dimension::percent(f),
        };
        let or_auto = |l: Length| match l {
            Length::Auto => LengthPercentageAuto::auto(),
            Length::Px(v) => LengthPercentageAuto::length(px(v)),
            Length::Fraction(f) => LengthPercentageAuto::percent(f),
        };
        let fixed = |v: f32| LengthPercentage::length(px(v));
        let overflow = match self.overflow {
            Overflow::Visible => TaffyOverflow::Visible,
            Overflow::Clip => TaffyOverflow::Clip,
            Overflow::Scroll => TaffyOverflow::Scroll,
        };
        taffy::Style {
            display: if self.hidden {
                Display::None
            } else {
                Display::Flex
            },
            flex_direction: match self.direction {
                Direction::Row => FlexDirection::Row,
                Direction::Column => FlexDirection::Column,
            },
            flex_wrap: if self.wrap {
                FlexWrap::Wrap
            } else {
                FlexWrap::NoWrap
            },
            size: taffy::Size {
                width: dimension(self.width),
                height: dimension(self.height),
            },
            min_size: taffy::Size {
                width: or_auto(self.min_width),
                height: or_auto(self.min_height),
            },
            max_size: taffy::Size {
                width: or_auto(self.max_width),
                height: or_auto(self.max_height),
            },
            flex_grow: self.grow,
            flex_shrink: self.shrink,
            flex_basis: dimension(self.basis),
            padding: taffy::Rect {
                left: fixed(self.padding.left),
                right: fixed(self.padding.right),
                top: fixed(self.padding.top),
                bottom: fixed(self.padding.bottom),
            },
            margin: taffy::Rect {
                left: or_auto(self.margin.left),
                right: or_auto(self.margin.right),
                top: or_auto(self.margin.top),
                bottom: or_auto(self.margin.bottom),
            },
            gap: taffy::Size {
                width: fixed(self.gap),
                height: fixed(self.gap),
            },
            align_items: align(self.align_items),
            align_self: self.align_self.map(align),
            justify_content: justify(self.justify),
            align_content: AlignContent::START,
            position: match self.position {
                Position::InFlow => TaffyPosition::Relative,
                Position::Absolute => TaffyPosition::Absolute,
            },
            inset: taffy::Rect {
                left: or_auto(self.inset.left),
                right: or_auto(self.inset.right),
                top: or_auto(self.inset.top),
                bottom: or_auto(self.inset.bottom),
            },
            overflow: taffy::Point {
                x: overflow,
                y: overflow,
            },
            scrollbar_width: 0.0,
            aspect_ratio: self.aspect_ratio,
            ..taffy::Style::default()
        }
    }
}

fn align(align: Align) -> AlignItems {
    match align {
        Align::Stretch => AlignItems::STRETCH,
        Align::Start => AlignItems::FLEX_START,
        Align::Center => AlignItems::CENTER,
        Align::End => AlignItems::FLEX_END,
    }
}

fn justify(justify: Justify) -> AlignContent {
    match justify {
        Justify::Start => AlignContent::FLEX_START,
        Justify::Center => AlignContent::CENTER,
        Justify::End => AlignContent::FLEX_END,
        Justify::SpaceBetween => AlignContent::SPACE_BETWEEN,
        Justify::SpaceAround => AlignContent::SPACE_AROUND,
    }
}
