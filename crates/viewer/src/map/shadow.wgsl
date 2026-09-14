// `shadowblur.shader`: the `ShadowBlurHorizontal` and `ShadowBlurVertical`
// effects, which the engine runs twice over the screen-space shadow buffer
// before any lit pass samples it.
//
// This file is compiled without `common.wgsl` ahead of it, like `post.wgsl`:
// the blur reads no frame constants, only its own offsets and weights.

/// `ConstantBuffer( 0, 0 ) { float4 vGaussianOffsetsWeights[2]; }`.
///
/// `vOffset[i]` is `offsets[i]` and `vWeight[i]` is `weights[i]`; the shader
/// never reads `vOffset[0]`, because the centre tap is taken from the
/// unoffset coordinate it already has.
struct Blur {
    /// `(vOffset[0], vOffset[1], vOffset[2], vWeight[0])`, the offsets in
    /// texture coordinates along the axis this pass blurs.
    offsets_weight0: vec4<f32>,
    /// `(vWeight[1], vWeight[2], unused, unused)`.
    weights: vec4<f32>,
}

@group(0) @binding(0) var<uniform> blur: Blur;
@group(0) @binding(1) var blur_sample: texture_2d<f32>;
@group(0) @binding(2) var blur_sampler: sampler;

struct FullScreen {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

/// `VertexShader`, which passes a screen quad through untouched. One oversized
/// triangle stands in for the quad, as it does in `post.wgsl`.
@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> FullScreen {
    var out: FullScreen;
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return out;
}

/// The body both `HorizontalPixelShader` and `VerticalPixelShader` carry; they
/// differ only in whether the offset goes into `x` or `y`.
fn blur_along(uv: vec2<f32>, axis: vec2<f32>) -> vec4<f32> {
    var offset = array<f32, 3>(
        blur.offsets_weight0.x,
        blur.offsets_weight0.y,
        blur.offsets_weight0.z,
    );
    var weight = array<f32, 3>(
        blur.offsets_weight0.w,
        blur.weights.x,
        blur.weights.y,
    );
    let original = textureSampleLevel(blur_sample, blur_sampler, uv, 0.0);
    var accum = original.r * weight[0];
    for (var i = 1; i < 3; i++) {
        accum += textureSampleLevel(blur_sample, blur_sampler, uv + axis * offset[i], 0.0).r
            * weight[i];
        accum += textureSampleLevel(blur_sample, blur_sampler, uv - axis * offset[i], 0.0).r
            * weight[i];
    }
    // Only the shadow is blurred; the two fog factors the pre-pass wrote pass
    // through from the centre tap, and the alpha is rewritten to 1.
    return vec4<f32>(accum, original.g, original.b, 1.0);
}

@fragment
fn horizontal_main(input: FullScreen) -> @location(0) vec4<f32> {
    return blur_along(input.uv, vec2<f32>(1.0, 0.0));
}

@fragment
fn vertical_main(input: FullScreen) -> @location(0) vec4<f32> {
    return blur_along(input.uv, vec2<f32>(0.0, 1.0));
}
