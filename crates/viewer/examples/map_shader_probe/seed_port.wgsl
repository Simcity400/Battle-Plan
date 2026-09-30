// Original post.wgsl bright/log seed, kept as an offline comparison oracle.
@group(0) @binding(0) var<uniform> bloom: vec4<f32>;
@group(0) @binding(1) var scene: texture_2d<f32>;
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
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return out;
}
@fragment
fn seed_main(input: FullScreen) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(scene, linear_clamp, input.uv, 0.0);
    var bright = max(0.0, max(max(color.r, color.g), color.b) - bloom.x);
    bright = bright / (0.5 + bright);
    bright = bright + color.a * bloom.y;
    let log_luminance = log(max(0.0, dot(color.rgb, vec3<f32>(0.2125, 0.7154, 0.0721))) + 0.0001);
    return vec4<f32>(color.rgb * bright, log_luminance);
}
