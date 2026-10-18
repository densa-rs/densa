//! Renders offscreen on the real GPU and reads pixels back: proves the pipeline, glyph
//! uploads into the atlas, UV orientation, blending and sRGB handling end to end.
//!
//! Needs a GPU adapter. Set `DENSA_SKIP_GPU_TESTS=1` on machines without one; otherwise a
//! missing adapter is a failure, not a silent pass.

use densa_render::{
    srgb, Align, AtlasError, CellStyle, CellText, Column, GpuGrid, GridPainter, GridSource,
    Instance, InstanceBuffer, Palette,
};
use densa_text::{AtlasConfig, AtlasUpdate, Font, GlyphUpload};
use std::sync::{Mutex, MutexGuard, PoisonError};

// Target widths are multiples of 64 px: 64 * 4 bytes = 256, the copy row alignment.
const W: u32 = 64;
const H: u32 = 32;
const ATLAS: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const BLACK: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// Serializes GPU tests. Concurrent devices on one adapter are fine while tests pass, but a
/// test panicking mid-submission once left the others hung in the driver instead of failing.
static GPU: Mutex<()> = Mutex::new(());

/// A device for one test, plus the guard keeping other GPU tests out until it is dropped.
fn device() -> Option<(wgpu::Device, wgpu::Queue, MutexGuard<'static, ()>)> {
    if std::env::var_os("DENSA_SKIP_GPU_TESTS").is_some() {
        return None;
    }
    // A poisoned lock only means an earlier test failed; this one can still run.
    let guard = GPU.lock().unwrap_or_else(PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("no GPU adapter (set DENSA_SKIP_GPU_TESTS=1 to skip GPU tests)");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("failed to create device");
    Some((device, queue, guard))
}

/// RGBA8 (sRGB-encoded) pixels read back from a render target, row-major.
struct Pixels {
    width: u32,
    bytes: Vec<u8>,
}

impl Pixels {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.bytes[i],
            self.bytes[i + 1],
            self.bytes[i + 2],
            self.bytes[i + 3],
        ]
    }

    /// Brightest red channel inside `[x0, x1) x [y0, y1)`.
    fn peak(&self, (x0, x1): (u32, u32), (y0, y1): (u32, u32)) -> u8 {
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .map(|(x, y)| self.at(x, y)[0])
            .max()
            .unwrap_or(0)
    }
}

/// Draws `instances` with `grid` over black into a `size` target and reads it back.
fn render_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    grid: &GpuGrid,
    size: (u32, u32),
    instances: &[Instance],
) -> Pixels {
    let (width, height) = size;
    assert!(width.is_multiple_of(64), "copy row alignment");
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(width * height * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let count = grid.prepare(queue, size, instances);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    grid.render(&mut encoder, &view, BLACK, count);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map failed"));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("poll failed");
    let bytes = slice.get_mapped_range().expect("mapped range").to_vec();
    readback.unmap();
    Pixels { width, bytes }
}

/// Draws `instances` into a `W` x `H` target with a fresh grid and an empty atlas.
fn render(device: &wgpu::Device, queue: &wgpu::Queue, instances: &[Instance]) -> Pixels {
    let grid = GpuGrid::new(device, FORMAT, 64);
    render_with(device, queue, &grid, (W, H), instances)
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 2)
}

/// `width` x `height` of coverage at (`x`, `y`): top row opaque, the rest transparent.
fn block(x: u32, y: u32, width: u32, height: u32) -> AtlasUpdate {
    let mut coverage = vec![0u8; (width * height) as usize];
    coverage[..width as usize].fill(255);
    AtlasUpdate::Write(GlyphUpload {
        x,
        y,
        width,
        height,
        coverage,
    })
}

const fn resize(width: u32, height: u32) -> AtlasUpdate {
    AtlasUpdate::Resize { width, height }
}

/// A white glyph quad at screen `(x, y)` showing atlas texels `[x0, y0, x1, y1]`.
fn glyph_at(x: f32, y: f32, texels: [f32; 4]) -> Instance {
    let size = [texels[2] - texels[0], texels[3] - texels[1]];
    Instance::glyph([x, y, size[0], size[1]], texels, srgb(0xFFFFFF, 1.0))
}

#[test]
fn solid_quads_land_on_the_right_pixels() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let red = srgb(0xFF0000, 1.0);
    let pixels = render(
        &device,
        &queue,
        &[Instance::solid([0.0, 0.0, 32.0, 32.0], red)],
    );
    assert!(
        close(pixels.at(10, 10), [255, 0, 0, 255]),
        "solid quad: {:?}",
        pixels.at(10, 10)
    );
    assert!(
        close(pixels.at(60, 30), [0, 0, 0, 255]),
        "clear color: {:?}",
        pixels.at(60, 30)
    );
}

#[test]
fn written_coverage_is_sampled_texel_for_pixel() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let mut grid = GpuGrid::new(&device, FORMAT, 64);
    let written = grid
        .apply_atlas_updates(&device, &queue, [resize(ATLAS, ATLAS), block(8, 4, 4, 2)])
        .unwrap();
    assert_eq!(written, 1);

    let glyph = glyph_at(20.0, 10.0, [8.0, 4.0, 12.0, 6.0]);
    let pixels = render_with(&device, &queue, &grid, (W, H), &[glyph]);
    assert!(close(pixels.at(21, 10), [255; 4]), "opaque texel row");
    assert!(
        close(pixels.at(21, 11), [0, 0, 0, 255]),
        "transparent texel row"
    );
    assert!(
        close(pixels.at(19, 10), [0, 0, 0, 255]),
        "left of the glyph"
    );
}

#[test]
fn growing_the_atlas_keeps_existing_texels_in_place() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let mut grid = GpuGrid::new(&device, FORMAT, 64);
    grid.apply_atlas_updates(&device, &queue, [resize(ATLAS, ATLAS), block(8, 4, 4, 2)])
        .unwrap();
    grid.apply_atlas_updates(
        &device,
        &queue,
        [resize(ATLAS, 2 * ATLAS), block(8, ATLAS + 36, 4, 2)],
    )
    .unwrap();
    assert_eq!(grid.atlas_size(), (ATLAS, 2 * ATLAS));

    let old = glyph_at(10.0, 10.0, [8.0, 4.0, 12.0, 6.0]);
    let y = (ATLAS + 36) as f32;
    let new = glyph_at(40.0, 10.0, [8.0, y, 12.0, y + 2.0]);
    let pixels = render_with(&device, &queue, &grid, (W, H), &[old, new]);
    assert!(
        close(pixels.at(11, 10), [255; 4]),
        "glyph from before the growth"
    );
    assert!(close(pixels.at(41, 10), [255; 4]), "glyph on the new page");
    assert!(close(pixels.at(11, 11), [0, 0, 0, 255]));
}

#[test]
fn smaller_resizes_keep_the_texture_and_its_texels() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let mut grid = GpuGrid::new(&device, FORMAT, 64);
    grid.apply_atlas_updates(
        &device,
        &queue,
        [resize(ATLAS, 2 * ATLAS), block(8, 4, 4, 2)],
    )
    .unwrap();
    // A fresh, smaller cache attached to this grid: its rects are absolute texels, so the
    // larger texture already serves them.
    grid.apply_atlas_updates(&device, &queue, [resize(ATLAS / 2, ATLAS / 2)])
        .unwrap();
    assert_eq!(grid.atlas_size(), (ATLAS, 2 * ATLAS));
    let pixels = render_with(
        &device,
        &queue,
        &grid,
        (W, H),
        &[glyph_at(20.0, 10.0, [8.0, 4.0, 12.0, 6.0])],
    );
    assert!(close(pixels.at(21, 10), [255; 4]));
}

#[test]
fn interleaved_resizes_and_writes_in_one_batch_all_land() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let mut grid = GpuGrid::new(&device, FORMAT, 64);
    let batch = [
        resize(ATLAS, ATLAS),
        block(8, 4, 4, 2),
        resize(ATLAS, 2 * ATLAS),
        block(8, ATLAS + 4, 4, 2),
        resize(ATLAS, 3 * ATLAS),
        block(8, 2 * ATLAS + 4, 4, 2),
    ];
    assert_eq!(grid.apply_atlas_updates(&device, &queue, batch).unwrap(), 3);
    let quads: Vec<Instance> = (0..3u32)
        .map(|page| {
            let y = (page * ATLAS + 4) as f32;
            glyph_at(10.0 + 20.0 * page as f32, 10.0, [8.0, y, 12.0, y + 2.0])
        })
        .collect();
    let pixels = render_with(&device, &queue, &grid, (W, H), &quads);
    for page in 0..3 {
        let x = 11 + 20 * page;
        assert!(close(pixels.at(x, 10), [255; 4]), "write on page {page}");
        assert!(
            close(pixels.at(x, 11), [0, 0, 0, 255]),
            "page {page} bottom row"
        );
    }
}

#[test]
fn atlases_beyond_the_device_limit_are_an_error() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let limit = GpuGrid::max_atlas_side(&device);
    assert_eq!(limit, device.limits().max_texture_dimension_2d);
    let mut grid = GpuGrid::new(&device, FORMAT, 64);
    let err = grid
        .apply_atlas_updates(&device, &queue, [resize(64, limit + 1)])
        .unwrap_err();
    assert!(matches!(err, AtlasError::TooLarge { .. }), "{err}");
}

#[test]
fn an_atlas_failure_is_latched_and_the_grid_stops_drawing() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let mut grid = GpuGrid::new(&device, FORMAT, 64);
    let err = grid
        .apply_atlas_updates(
            &device,
            &queue,
            [
                resize(ATLAS, ATLAS),
                block(62, 0, 4, 2),
                resize(ATLAS, 2 * ATLAS),
            ],
        )
        .unwrap_err();
    assert!(matches!(err, AtlasError::BadWrite { .. }), "{err}");
    assert_eq!(
        grid.atlas_size(),
        (ATLAS, ATLAS),
        "updates after the error are not applied"
    );
    assert_eq!(grid.atlas_error(), Some(err));

    // Later updates, even valid ones, keep failing; nothing is drawn from a stale atlas.
    let again = grid.apply_atlas_updates(&device, &queue, [block(0, 0, 4, 2)]);
    assert_eq!(again, Err(err));
    let quad = glyph_at(20.0, 10.0, [8.0, 4.0, 12.0, 6.0]);
    assert_eq!(grid.prepare(&queue, (W, H), &[quad]), 0);
    let pixels = render_with(&device, &queue, &grid, (W, H), &[quad]);
    assert!(close(pixels.at(21, 10), [0, 0, 0, 255]), "nothing drawn");
}

#[test]
fn writes_whose_pixels_do_not_match_their_size_are_an_error() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let mut grid = GpuGrid::new(&device, FORMAT, 64);
    let short = AtlasUpdate::Write(GlyphUpload {
        x: 0,
        y: 0,
        width: 4,
        height: 4,
        coverage: vec![255; 3],
    });
    let err = grid
        .apply_atlas_updates(&device, &queue, [resize(ATLAS, ATLAS), short])
        .unwrap_err();
    assert!(
        matches!(err, AtlasError::BadWrite { bytes: 3, .. }),
        "{err}"
    );
}

/// One row: a Vietnamese label and a number.
struct OneRow;

impl GridSource for OneRow {
    fn rows(&self) -> usize {
        1
    }

    fn cell(&self, _row: usize, column: usize, text: &mut CellText) -> CellStyle {
        let _ = text.push_str(["Hà", "1234.5"][column]);
        CellStyle {
            color: srgb(0xFFFFFF, 1.0),
            background: None,
        }
    }
}

#[test]
fn painted_text_shows_up_as_ink_inside_its_cells() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let columns = vec![
        Column::new("A", 4, Align::Left),
        Column::new("B", 8, Align::Right),
    ];
    let font = Font::bundled_mono().unwrap();
    let mut painter = GridPainter::new(columns, 1, font, 14.0, Palette::new());
    let layout = painter.layout().clone();
    let mut instances = InstanceBuffer::with_capacity(layout.max_instances());
    painter.paint(&OneRow, &mut instances);

    let mut grid = GpuGrid::new(&device, FORMAT, layout.max_instances());
    let written = grid
        .apply_atlas_updates(&device, &queue, painter.drain_updates())
        .unwrap();
    assert!(written > 0);
    assert_eq!(
        grid.atlas_size(),
        painter.atlas_extent(),
        "the GPU follows the cache"
    );
    let size = (
        layout.width().next_multiple_of(64),
        layout.height().next_multiple_of(64),
    );
    let pixels = render_with(&device, &queue, &grid, size, instances.as_slice());

    let rows = (layout.row_y(1), layout.row_y(1) + layout.row_height());
    for column in 0..2 {
        let x0 = layout.column_x(column);
        let columns = (x0, x0 + layout.column_width(column));
        assert!(
            pixels.peak(columns, rows) > 200,
            "no ink in column {column}"
        );
    }
    let gap = (
        layout.column_x(0) + layout.column_width(0) + 1,
        layout.column_x(1) - 1,
    );
    assert!(pixels.peak(gap, rows) < 60, "ink between the columns");
}

#[test]
fn text_drawn_after_the_atlas_grew_mid_frame_still_lands_in_its_cells() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let columns = vec![
        Column::new("A", 4, Align::Left),
        Column::new("B", 8, Align::Right),
    ];
    let font = Font::bundled_mono().unwrap();
    // 32 x 32 pages hold only a few glyphs: this frame forces several new pages.
    let atlas = AtlasConfig::new(32, 16);
    let mut painter = GridPainter::new(columns, 1, font, 14.0, Palette::new()).with_atlas(atlas);
    let layout = painter.layout().clone();
    let mut instances = InstanceBuffer::with_capacity(layout.max_instances());
    painter.paint(&OneRow, &mut instances);
    assert!(
        painter.atlas_extent().1 > 32,
        "the frame must grow the atlas"
    );
    assert_eq!(painter.cache_stats().dropped, 0);

    let mut grid = GpuGrid::new(&device, FORMAT, layout.max_instances());
    grid.apply_atlas_updates(&device, &queue, painter.drain_updates())
        .unwrap();
    assert_eq!(grid.atlas_size(), painter.atlas_extent());
    let size = (
        layout.width().next_multiple_of(64),
        layout.height().next_multiple_of(64),
    );
    let pixels = render_with(&device, &queue, &grid, size, instances.as_slice());
    let rows = (layout.row_y(1), layout.row_y(1) + layout.row_height());
    for column in 0..2 {
        let x0 = layout.column_x(column);
        let columns = (x0, x0 + layout.column_width(column));
        assert!(
            pixels.peak(columns, rows) > 200,
            "no ink in column {column}"
        );
    }
}

#[test]
fn translucent_quads_blend_over_the_background() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let half_white = srgb(0xFFFFFF, 0.5);
    let pixels = render(
        &device,
        &queue,
        &[Instance::solid([0.0, 0.0, W as f32, H as f32], half_white)],
    );
    // 50% linear white over black is linear 0.5, which sRGB-encodes to ~188.
    let [r, g, b, _] = pixels.at(5, 5);
    assert!(
        [r, g, b].iter().all(|&c| c.abs_diff(188) <= 3),
        "{r} {g} {b}"
    );
}

#[test]
fn instances_beyond_gpu_capacity_are_clipped_not_overrun() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let grid = GpuGrid::new(&device, FORMAT, 2);
    let quad = Instance::solid([0.0; 4], [1.0; 4]);
    assert_eq!(grid.prepare(&queue, (W, H), &[quad; 5]), 2);
    assert_eq!(grid.capacity(), 2);
}
