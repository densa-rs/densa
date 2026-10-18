use bytemuck::{Pod, Zeroable};

/// One quad, as read by the vertex shader (per-instance step mode).
///
/// A glyph has a non-empty `uv` rectangle; a solid rectangle has `uv` all zero.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Instance {
    /// `[x, y, width, height]` in physical pixels, origin top-left.
    pub rect: [f32; 4],
    /// `[u0, v0, u1, v1]` in the glyph atlas.
    pub uv: [f32; 4],
    /// Linear RGBA, not premultiplied.
    pub color: [f32; 4],
}

impl Instance {
    /// A filled rectangle.
    pub const fn solid(rect: [f32; 4], color: [f32; 4]) -> Self {
        Self {
            rect,
            uv: [0.0; 4],
            color,
        }
    }

    /// A glyph: `uv` selects its atlas cell, `color` tints its coverage.
    pub const fn glyph(rect: [f32; 4], uv: [f32; 4], color: [f32; 4]) -> Self {
        Self { rect, uv, color }
    }

    /// `true` for glyphs, `false` for solid rectangles.
    pub fn is_glyph(&self) -> bool {
        self.uv[2] > self.uv[0]
    }
}

/// Fixed-capacity instance list rebuilt every frame without allocating.
///
/// Pushing past capacity drops the instance and counts it, so an undersized buffer is
/// visible in metrics instead of silently growing (and allocating) on the hot path.
#[derive(Debug, Clone)]
pub struct InstanceBuffer {
    items: Box<[Instance]>,
    len: usize,
    dropped: u64,
}

impl InstanceBuffer {
    /// An empty buffer holding up to `capacity` instances. Allocates once, here.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            items: vec![Instance::zeroed(); capacity].into_boxed_slice(),
            len: 0,
            dropped: 0,
        }
    }

    /// Starts a new frame.
    pub fn clear(&mut self) {
        self.len = 0;
        self.dropped = 0;
    }

    /// Appends `instance`; returns `false` (and counts a drop) when full.
    #[inline]
    pub fn push(&mut self, instance: Instance) -> bool {
        match self.items.get_mut(self.len) {
            Some(slot) => {
                *slot = instance;
                self.len += 1;
                true
            }
            None => {
                self.dropped += 1;
                false
            }
        }
    }

    /// Instances pushed since the last [`clear`](Self::clear).
    pub fn as_slice(&self) -> &[Instance] {
        &self.items[..self.len]
    }

    /// Number of instances pushed.
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` if nothing was pushed.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Maximum number of instances.
    pub fn capacity(&self) -> usize {
        self.items.len()
    }

    /// Instances dropped since the last [`clear`](Self::clear).
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}
