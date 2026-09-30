// Retired post.wgsl reduction kernels, retained only as an offline oracle.
@group(0) @binding(0) var<uniform> reduction_uv: vec4<f32>;
@group(0) @binding(1) var luminance: texture_2d<f32>;
@group(0) @binding(2) var linear_clamp: sampler;
struct FullScreen {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> FullScreen {
    var out: FullScreen;
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5) * reduction_uv.xy;
    return out;
}
@fragment
fn reduce_seed_main(input: FullScreen) -> @location(0) f32 {
    return textureSampleLevel(luminance, linear_clamp, input.uv, 0.0).a;
}
@fragment
fn reduce_main(input: FullScreen) -> @location(0) f32 {
    return textureSampleLevel(luminance, linear_clamp, input.uv, 0.0).r;
}
