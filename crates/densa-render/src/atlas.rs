use densa_text::{AtlasUpdate, GlyphUpload};
use std::fmt;

/// Why the GPU atlas could not follow the glyph cache.
///
/// Every variant means the texture no longer mirrors the cache (a bug or an unsupported
/// device). `GpuRenderer` latches the first one: it then draws nothing and keeps returning it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AtlasError {
    /// The cache grew beyond the device's 2D texture limit.
    TooLarge {
        /// Requested width in texels.
        width: u32,
        /// Requested height in texels.
        height: u32,
        /// `max_texture_dimension_2d` of the device.
        limit: u32,
    },
    /// A write does not lie inside the texture, or its pixel count does not match its size.
    BadWrite {
        /// The write's `(x, y, width, height)`.
        rect: (u32, u32, u32, u32),
        /// Bytes supplied.
        bytes: usize,
        /// Current texture `(width, height)`.
        atlas: (u32, u32),
    },
    /// An update kind this renderer does not know (from a newer `densa-text`).
    UnsupportedUpdate,
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge {
                width,
                height,
                limit,
            } => write!(
                f,
                "glyph atlas of {width}x{height} exceeds the device's texture limit {limit}"
            ),
            Self::BadWrite { rect, bytes, atlas } => write!(
                f,
                "glyph write {rect:?} with {bytes} bytes does not fit the {atlas:?} atlas"
            ),
            Self::UnsupportedUpdate => f.write_str("unsupported glyph atlas update"),
        }
    }
}

impl std::error::Error for AtlasError {}

/// The R8 coverage texture mirroring a `densa_text::GlyphCache`.
#[derive(Debug)]
pub(crate) struct GpuAtlas {
    texture: wgpu::Texture,
    /// Increments whenever `texture` is replaced.
    version: u64,
}

impl GpuAtlas {
    /// A 1x1 placeholder until the cache's first [`AtlasUpdate::Resize`] arrives.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        Self {
            texture: create_texture(device, 1, 1),
            version: 0,
        }
    }

    /// Changes whenever the texture is replaced: views and bind groups of an older version
    /// are stale.
    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }

    pub(crate) fn view(&self) -> wgpu::TextureView {
        self.texture
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Applies `updates` in order; returns how many writes were applied.
    pub(crate) fn apply(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        updates: impl IntoIterator<Item = AtlasUpdate>,
    ) -> Result<usize, AtlasError> {
        let mut writes = 0;
        for update in updates {
            match update {
                AtlasUpdate::Resize { width, height } => {
                    self.grow(device, queue, width, height)?;
                }
                AtlasUpdate::Write(upload) => {
                    self.write(queue, &upload)?;
                    writes += 1;
                }
                _ => return Err(AtlasError::UnsupportedUpdate),
            }
        }
        Ok(writes)
    }

    /// Makes the texture at least `width` x `height`, replacing it by a larger one holding
    /// the same texels at the same place. Never shrinks: a smaller request (a fresh cache
    /// attached to a grown texture) is satisfied already, since rects are absolute texels.
    fn grow(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<(), AtlasError> {
        let current = self.size();
        let (width, height) = (width.max(current.0), height.max(current.1));
        if (width, height) == current {
            return Ok(());
        }
        let limit = device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            return Err(AtlasError::TooLarge {
                width,
                height,
                limit,
            });
        }

        let grown = create_texture(device, width, height);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("densa atlas grow"),
        });
        encoder.copy_texture_to_texture(
            self.texture.as_image_copy(),
            grown.as_image_copy(),
            self.texture.size(),
        );
        // Submitted now: writes already queued for the old texture run first (queue writes
        // execute before the next submission), and writes after this target the new one.
        queue.submit([encoder.finish()]);
        self.texture = grown;
        self.version += 1;
        Ok(())
    }

    fn write(&self, queue: &wgpu::Queue, upload: &GlyphUpload) -> Result<(), AtlasError> {
        let (width, height) = self.size();
        let inside = upload
            .x
            .checked_add(upload.width)
            .is_some_and(|r| r <= width)
            && upload
                .y
                .checked_add(upload.height)
                .is_some_and(|b| b <= height);
        let complete =
            upload.coverage.len() as u64 == u64::from(upload.width) * u64::from(upload.height);
        if !inside || !complete {
            return Err(AtlasError::BadWrite {
                rect: (upload.x, upload.y, upload.width, upload.height),
                bytes: upload.coverage.len(),
                atlas: (width, height),
            });
        }
        if upload.width == 0 || upload.height == 0 {
            return Ok(());
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: upload.x,
                    y: upload.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &upload.coverage,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(upload.width),
                rows_per_image: Some(upload.height),
            },
            wgpu::Extent3d {
                width: upload.width,
                height: upload.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }
}

/// An R8 coverage texture; wgpu zero-initializes it, so unwritten texels are transparent.
fn create_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("densa glyph atlas"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
