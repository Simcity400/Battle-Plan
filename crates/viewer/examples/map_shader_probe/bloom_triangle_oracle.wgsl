// Attribution oracle: old bloom arithmetic with native triangle order. Never linked by the viewer.

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
    return out;
}

/// `bloom.shader`: a centre tap and four linearly filtered pairs along one axis.
@fragment
fn blur_main(input: FullScreen) -> @location(0) vec4<f32> {
    let step = blur.inv_size * vec2<f32>(blur.axis, 1.0 - blur.axis);
    var color = textureSampleLevel(luminance, linear_clamp, input.uv, 0.0).rgb * blur.weight0;
    for (var k = 0; k < 4; k = k + 1) {
        let offset = step * blur.offsets[k];
        color = color + blur.weights[k] * (
            textureSampleLevel(luminance, linear_clamp, input.uv + offset, 0.0).rgb
            + textureSampleLevel(luminance, linear_clamp, input.uv - offset, 0.0).rgb);
    }
    return vec4<f32>(color, 1.0);
}

