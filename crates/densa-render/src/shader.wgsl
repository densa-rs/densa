// Instanced quads: each instance is a solid rectangle or a glyph from the R8 atlas.

struct Globals {
    // Render target size in physical pixels.
    screen: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    // 1.0 for glyphs, 0.0 for solid rectangles.
    @location(2) textured: f32,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
) -> VertexOut {
    // Triangle strip corners: (0,0) (1,0) (0,1) (1,1).
    let corner = vec2<f32>(f32(vertex & 1u), f32(vertex >> 1u));
    let pixel = rect.xy + corner * rect.zw;
    let ndc = vec2<f32>(
        pixel.x / globals.screen.x * 2.0 - 1.0,
        1.0 - pixel.y / globals.screen.y * 2.0,
    );

    var out: VertexOut;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    // UVs arrive in atlas texels, so they stay valid when the atlas grows.
    let atlas_size = vec2<f32>(textureDimensions(atlas));
    out.uv = mix(uv.xy, uv.zw, corner) / atlas_size;
    out.color = color;
    out.textured = select(0.0, 1.0, uv.z > uv.x);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Sampled unconditionally to keep texture sampling in uniform control flow.
    let coverage = textureSample(atlas, atlas_sampler, in.uv).r;
    let alpha = mix(1.0, coverage, in.textured);
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}
