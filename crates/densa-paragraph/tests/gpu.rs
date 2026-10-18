//! Draws laid-out paragraphs on the real GPU and reads the pixels back.
//!
//! Needs a GPU adapter. Set `DENSA_SKIP_GPU_TESTS=1` on machines without one; otherwise a
//! missing adapter is a failure, not a silent pass.

use densa_draw::{Canvas, InstanceBuffer};
use densa_paragraph::{Color, Paragraph, ParagraphStyle, Point, TextAlign, TextSystem};
use densa_render::GpuRenderer;
use densa_text::{AtlasConfig, GlyphCache};
use std::sync::{Mutex, MutexGuard, PoisonError};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const W: u32 = 320; // a multiple of 64: rows of 4-byte pixels meet the 256-byte alignment
const H: u32 = 128;
const ORIGIN: Point = Point::new(8.0, 8.0);

/// Serializes GPU tests: a test panicking mid-submission must not hang the others.
static GPU: Mutex<()> = Mutex::new(());

fn device() -> Option<(wgpu::Device, wgpu::Queue, MutexGuard<'static, ()>)> {
    if std::env::var_os("DENSA_SKIP_GPU_TESTS").is_some() {
        return None;
    }
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

/// Red channel of a `W` x `H` RGBA8 frame showing `paragraph` in white at `ORIGIN`.
fn render(device: &wgpu::Device, queue: &wgpu::Queue, paragraph: &Paragraph) -> Vec<u8> {
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let mut instances = InstanceBuffer::with_capacity(paragraph.glyph_count());
    let mut canvas = Canvas::begin(&mut cache, &mut instances);
    for run in paragraph.runs() {
        canvas.draw_run(run, ORIGIN);
    }

    let mut grid = GpuRenderer::new(device, FORMAT, instances.capacity().max(1));
    grid.apply_atlas_updates(device, queue, cache.drain_updates())
        .unwrap();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("paragraph target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(W * H * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let count = grid.prepare(queue, (W, H), instances.as_slice());
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    grid.render(&mut encoder, &view, Color::BLACK, count);
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 4),
                rows_per_image: Some(H),
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
    let rgba = slice.get_mapped_range().expect("mapped range").to_vec();
    let (pixels, _) = rgba.as_chunks::<4>();
    pixels.iter().map(|px| px[0]).collect()
}

/// Columns holding ink (red above `threshold`) between rows `y0..y1`.
fn inked_columns(red: &[u8], (y0, y1): (u32, u32)) -> Vec<u32> {
    (0..W)
        .filter(|&x| (y0..y1).any(|y| red[(y * W + x) as usize] > 128))
        .collect()
}

fn line_rows(paragraph: &Paragraph, line: usize) -> (u32, u32) {
    let l = &paragraph.lines()[line];
    // Lossless: small positive pixel coordinates.
    ((ORIGIN.y + l.top) as u32, (ORIGIN.y + l.bottom) as u32)
}

#[test]
fn wrapped_lines_are_drawn_inside_their_boxes_and_the_width() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let max_width = 120.0;
    let style = ParagraphStyle::new(14.0).with_max_width(Some(max_width));
    let mut system = TextSystem::new().unwrap();
    let paragraph = system
        .paragraph("Ünïcödé text wraps onto several lines here", &style, &[])
        .unwrap();
    assert!(paragraph.lines().len() >= 3);
    let red = render(&device, &queue, &paragraph);

    for line in 0..paragraph.lines().len() {
        let inked = inked_columns(&red, line_rows(&paragraph, line));
        assert!(!inked.is_empty(), "line {line} has no ink");
        let right = *inked.last().unwrap() as f32;
        assert!(
            right <= ORIGIN.x + max_width + 1.0,
            "line {line} ink at {right}"
        );
    }
    let below = ((ORIGIN.y + paragraph.height()) as u32 + 4, H);
    assert!(
        inked_columns(&red, below).is_empty(),
        "ink below the last line"
    );
}

#[test]
fn centered_lines_are_drawn_around_the_middle() {
    let Some((device, queue, _gpu)) = device() else {
        return;
    };
    let max_width = 300.0;
    let style = ParagraphStyle::new(14.0)
        .with_max_width(Some(max_width))
        .with_align(TextAlign::Center);
    let mut system = TextSystem::new().unwrap();
    let paragraph = system.paragraph("centre", &style, &[]).unwrap();
    let red = render(&device, &queue, &paragraph);

    let inked = inked_columns(&red, line_rows(&paragraph, 0));
    let (left, right) = (inked[0] as f32, *inked.last().unwrap() as f32);
    let middle = ORIGIN.x + max_width / 2.0;
    assert!(
        ((left + right) / 2.0 - middle).abs() < 4.0,
        "ink {left}..{right}"
    );
}
