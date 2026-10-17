//! Renders offscreen on the real GPU and reads pixels back: proves the pipeline, atlas
//! upload, UV orientation, blending and sRGB handling end to end.
//!
//! Needs a GPU adapter. Set `DENSA_SKIP_GPU_TESTS=1` on machines without one; otherwise a
//! missing adapter is a failure, not a silent pass.

use densa_render::{srgb, GlyphAtlas, GpuGrid, Instance, GLYPH_PX};

const W: u32 = 64; // 64 px * 4 bytes = 256: satisfies the copy row alignment.
const H: u32 = 32;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

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

/// Renders `instances` over `clear` and returns RGBA8 (sRGB-encoded) pixels.
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    instances: &[Instance],
    clear: wgpu::Color,
) -> Vec<u8> {
    let grid = GpuGrid::new(device, queue, FORMAT, 64);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test target"),
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
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(W * H * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let count = grid.prepare(queue, (W, H), instances);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    grid.render(&mut encoder, &view, clear, count);
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
    let pixels = slice.get_mapped_range().expect("mapped range").to_vec();
    readback.unmap();
    pixels
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 2)
}

#[test]
fn solid_quads_and_glyphs_land_on_the_right_pixels() {
    let Some((device, queue)) = device() else {
        return;
    };
    let atlas = GlyphAtlas::font8x8();
    let red = srgb(0xFF0000, 1.0);
    let white = srgb(0xFFFFFF, 1.0);
    // Glyph 'A' drawn at 2x (16x16 px) at x = 32.
    let scale = 2.0;
    let glyph_size = GLYPH_PX as f32 * scale;
    let instances = [
        Instance::solid([0.0, 0.0, 32.0, 32.0], red),
        Instance::glyph([32.0, 0.0, glyph_size, glyph_size], atlas.uv(b'A'), white),
    ];
    let black = wgpu::Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    let pixels = render(&device, &queue, &instances, black);

    assert!(
        close(pixel(&pixels, 10, 10), [255, 0, 0, 255]),
        "solid quad: {:?}",
        pixel(&pixels, 10, 10)
    );
    assert!(
        close(pixel(&pixels, 60, 30), [0, 0, 0, 255]),
        "clear color: {:?}",
        pixel(&pixels, 60, 30)
    );

    // 'A' row 0 = pixels 2 and 3 lit; at 2x they cover x in [36, 40), y in [0, 2).
    assert!(
        close(pixel(&pixels, 37, 1), [255, 255, 255, 255]),
        "lit glyph texel"
    );
    assert!(
        close(pixel(&pixels, 33, 1), [0, 0, 0, 255]),
        "unlit glyph texel"
    );
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
        wgpu::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
    );
    // 50% linear white over black is linear 0.5, which sRGB-encodes to ~188.
    let [r, g, b, _] = pixel(&pixels, 5, 5);
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
    let grid = GpuGrid::new(&device, &queue, FORMAT, 2);
    let quad = Instance::solid([0.0; 4], [1.0; 4]);
    assert_eq!(grid.prepare(&queue, (W, H), &[quad; 5]), 2);
    assert_eq!(grid.capacity(), 2);
}
