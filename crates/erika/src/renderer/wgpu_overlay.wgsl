// WGSL port of the Metal overlay shader in `renderer/metal/apple.rs`. Draws a
// textured quad placed by pixel rect within the viewport, alpha-blended over the
// video plane. Mode 0 samples straight RGBA; mode 1 is an alpha mask tinted by
// `color` (libass coverage bitmaps).

struct OverlayUniforms {
    rect: vec4<f32>,
    tex_rect: vec4<f32>,
    viewport: vec2<f32>,
    overlay_mode: u32,
    reserved0: u32,
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: OverlayUniforms;
@group(0) @binding(1) var overlay_texture: texture_2d<f32>;
@group(0) @binding(2) var overlay_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

@vertex
fn erika_overlay_vertex(@builtin(vertex_index) vertex_id: u32) -> VertexOut {
    var unit_positions = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    var tex_coords = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );

    let pixel = uniforms.rect.xy + unit_positions[vertex_id] * uniforms.rect.zw;
    let ndc = vec2<f32>(
        pixel.x / max(uniforms.viewport.x, 1.0) * 2.0 - 1.0,
        1.0 - pixel.y / max(uniforms.viewport.y, 1.0) * 2.0,
    );

    var out: VertexOut;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.tex_coord = uniforms.tex_rect.xy + tex_coords[vertex_id] * uniforms.tex_rect.zw;
    return out;
}

@fragment
fn erika_overlay_fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let sampled = textureSample(overlay_texture, overlay_sampler, in.tex_coord);
    if (uniforms.overlay_mode == 1u) {
        return vec4<f32>(uniforms.color.rgb, uniforms.color.a * sampled.r);
    }
    return sampled;
}
