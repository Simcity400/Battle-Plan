// `restorescene.shader`'s `RestoreBloom` effect, with the `downsample*.shader`
// gather that feeds its `AverageLuminanceTexture`. Seed, reductions and bloom are compiled from content.
//
// This file is self-contained: it is compiled without `common.wgsl` ahead of it
// because the post stage reads no frame constants.

struct Post {
    /// `(MiddleGrey, hdr min, hdr max, luminance adaptation blend)`.
    grade: vec4<f32>,
    /// `(gather width, gather height, inverse backing width, inverse backing height)`.
    gather: vec4<f32>,
}

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var luminance: texture_2d<f32>;
@group(0) @binding(3) var previous_luminance: texture_2d<f32>;
@group(0) @binding(4) var color_cube: texture_2d<f32>;
@group(0) @binding(5) var linear_clamp: sampler;
@group(0) @binding(6) var<uniform> reduction_uv: vec4<f32>;
@group(0) @binding(7) var bloom_texture: texture_2d<f32>;

const CUBE_SIZE: f32 = 32.0;
/// `LEVELS` in `restorescene.shader`'s `LevelValue`.
const LEVELS: vec2<f32> = vec2<f32>(0.04, 0.80);
/// `HSV` and `ColorBalance`: the render singleton's constants (`140ff3980`),
/// never updated, so the date-driven `CSeasons` grades do not reach this pass.
const HSV: vec3<f32> = vec3<f32>(0.0, 1.0, 1.0);
const COLOR_BALANCE: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);

struct FullScreen {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

/// One oversized triangle, so there is no seam down the diagonal of a quad.
@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> FullScreen {
    var out: FullScreen;
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5) * reduction_uv.xy;
    return out;
}

/// `--postfx off`: the float scene written straight into the 8-bit output, so
/// the verification dump is the raw scene and the target's own clamp is the
/// only thing that touches it. No engine stage corresponds to this. It is what
/// the game's own scene target is compared against: the height-290 capture
/// dumps that target directly, and `map::textures` tabulates the two.
@fragment
fn copy_main(input: FullScreen) -> @location(0) vec4<f32> {
    return textureSampleLevel(scene, linear_clamp, input.uv, 0.0);
}

/// `clamp(exp(vSum), MinHdr, MaxHdr)` and `CalculateAdaptedLuminance`.
@fragment
fn adapt_main(input: FullScreen) -> @location(0) f32 {
    // `LuminanceGather`: sum the last active rectangle in row-major order.
    // Repeated float UV addition, backing stride and the final float division
    // are intentional; another rounded mip reduction is not the same operation.
    var sum = 0.0;
    var v = post.gather.w * 0.5;
    for (var y = 0u; y < u32(post.gather.y); y = y + 1u) {
        var u = post.gather.z * 0.5;
        for (var x = 0u; x < u32(post.gather.x); x = x + 1u) {
            sum = sum + textureSampleLevel(luminance, linear_clamp, vec2<f32>(u, v), 0.0).r;
            u = u + post.gather.z;
        }
        v = v + post.gather.w;
    }
    let mean = sum / (post.gather.x * post.gather.y);
    // HLSL `clamp` is `min(max(x, lo), hi)`, and the `green_night` volume ships
    // `hdr_min_adjustment 0.8` above `hdr_max_adjustment 0.6`, so the order
    // matters: written out rather than left to WGSL's `clamp`, which does not
    // define the inverted case.
    let current = min(max(exp(mean), post.grade.y), post.grade.z);
    let previous = textureLoad(previous_luminance, vec2<i32>(0, 0), 0).r;
    // `vLast + (vCurrent - vLast) * (1 - exp(-tau * dt))`, with the blend
    // supplied by the caller: 1 for a headless frame and a settled camera.
    return previous + (current - previous) * post.grade.w;
}

