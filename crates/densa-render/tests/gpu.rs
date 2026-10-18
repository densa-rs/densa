//! Renders offscreen on the real GPU and reads pixels back: proves the pipeline, glyph
//! uploads into the atlas, UV orientation, blending and sRGB handling end to end.
//!
//! Needs a GPU adapter. Set `DENSA_SKIP_GPU_TESTS=1` on machines without one; otherwise a
//! missing adapter is a failure, not a silent pass.

use densa_render::{
    srgb, Align, CellStyle, CellText, Column, GpuGrid, GridPainter, GridSource, Instance,
    InstanceBuffer, Palette,
};
use densa_text::{Font, GlyphUpload};

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

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    if std::env::var_os("DENSA_SKIP_GPU_TESTS").is_some() {
        return None;
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("no GPU adapter (set DENSA_SKIP_GPU_TESTS=1 to skip GPU tests)");
    Some(
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("failed to create device"),
    )
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
    let grid = GpuGrid::new(device, FORMAT, 64, ATLAS);
    render_with(device, queue, &grid, (W, H), instances)
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 2)
}

#[test]
fn solid_quads_land_on_the_right_pixels() {
    let Some((device, queue)) = device() else {
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
fn uploaded_coverage_is_sampled_texel_for_pixel() {
    let Some((device, queue)) = device() else {
        return;
    };
    let grid = GpuGrid::new(&device, FORMAT, 64, ATLAS);
    // A 4x2 block at atlas (8, 4): top row opaque, bottom row transparent.
    let upload = GlyphUpload {
        x: 8,
        y: 4,
        width: 4,
        height: 2,
        coverage: [[255u8; 4], [0u8; 4]].concat(),
    };
    assert_eq!(grid.upload_glyphs(&queue, [upload]), 1);

    let a = ATLAS as f32;
    let uv = [8.0 / a, 4.0 / a, 12.0 / a, 6.0 / a];
    let white = srgb(0xFFFFFF, 1.0);
    let glyph = Instance::glyph([20.0, 10.0, 4.0, 2.0], uv, white);
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
fn empty_uploads_are_skipped() {
    let Some((device, queue)) = device() else {
        return;
    };
    let grid = GpuGrid::new(&device, FORMAT, 64, ATLAS);
    let empty = GlyphUpload {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        coverage: Vec::new(),
    };
    assert_eq!(grid.upload_glyphs(&queue, [empty]), 0);
    assert_eq!(grid.atlas_size(), ATLAS);
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
    let Some((device, queue)) = device() else {
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

    let grid = GpuGrid::new(
        &device,
        FORMAT,
        layout.max_instances(),
        painter.atlas_size(),
    );
    assert!(grid.upload_glyphs(&queue, painter.drain_uploads()) > 0);
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
fn translucent_quads_blend_over_the_background() {
    let Some((device, queue)) = device() else {
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
    let Some((device, queue)) = device() else {
        return;
    };
    let grid = GpuGrid::new(&device, FORMAT, 2, ATLAS);
    let quad = Instance::solid([0.0; 4], [1.0; 4]);
    assert_eq!(grid.prepare(&queue, (W, H), &[quad; 5]), 2);
    assert_eq!(grid.capacity(), 2);
}
