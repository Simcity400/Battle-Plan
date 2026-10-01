// Attribution oracle: native triangle and interpolated offset UVs. Never linked by the viewer.

/// `bloom.shader`'s `ConstantBuffer(2, 39)`.
struct Blur {
    inv_size: vec2<f32>,
    axis: f32,
    weight0: f32,
    weights: vec4<f32>,
    offsets: vec4<f32>,
}

@group(0) @binding(2) var luminance: texture_2d<f32>;
@group(0) @binding(5) var linear_clamp: sampler;
@group(0) @binding(6) var<uniform> reduction_uv: vec4<f32>;
@group(0) @binding(8) var<uniform> blur: Blur;

struct FullScreen {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) pair0: vec4<f32>,
    @location(2) pair1: vec4<f32>,
    @location(3) pair2: vec4<f32>,
    @location(4) pair3: vec4<f32>,

}

/// One oversized triangle, so there is no seam down the diagonal of a quad.
@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> FullScreen {
    var out: FullScreen;
    let positions = array<vec2<f32>, 3>(vec2(3.0, -1.0), vec2(-1.0, -1.0), vec2(-1.0, 3.0));
    let x = positions[index].x;
    let y = positions[index].y;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5) * reduction_uv.xy;
    let step = blur.inv_size * vec2<f32>(blur.axis, 1.0 - blur.axis);
    out.pair0 = vec4<f32>(out.uv + step * blur.offsets[0], out.uv - step * blur.offsets[0]);
    out.pair1 = vec4<f32>(out.uv + step * blur.offsets[1], out.uv - step * blur.offsets[1]);
    out.pair2 = vec4<f32>(out.uv + step * blur.offsets[2], out.uv - step * blur.offsets[2]);
    out.pair3 = vec4<f32>(out.uv + step * blur.offsets[3], out.uv - step * blur.offsets[3]);
    return out;
}

/// `bloom.shader`: a centre tap and four linearly filtered pairs along one axis.
@fragment
fn blur_main(input: FullScreen) -> @location(0) vec4<f32> {
    var color = textureSampleLevel(luminance, linear_clamp, input.uv, 0.0).rgb * blur.weight0;
    color = color + blur.weights[0] * (textureSampleLevel(luminance, linear_clamp, input.pair0.xy, 0.0).rgb + textureSampleLevel(luminance, linear_clamp, input.pair0.zw, 0.0).rgb);
    color = color + blur.weights[1] * (textureSampleLevel(luminance, linear_clamp, input.pair1.xy, 0.0).rgb + textureSampleLevel(luminance, linear_clamp, input.pair1.zw, 0.0).rgb);
    color = color + blur.weights[2] * (textureSampleLevel(luminance, linear_clamp, input.pair2.xy, 0.0).rgb + textureSampleLevel(luminance, linear_clamp, input.pair2.zw, 0.0).rgb);
    color = color + blur.weights[3] * (textureSampleLevel(luminance, linear_clamp, input.pair3.xy, 0.0).rgb + textureSampleLevel(luminance, linear_clamp, input.pair3.zw, 0.0).rgb);
    return vec4<f32>(color, 1.0);
}
