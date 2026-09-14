// `restorescene.shader`'s `Restore` effect, with the two `downsample*.shader`
// stages that feed its `AverageLuminanceTexture`. `BLOOM` is not ported, so the
// bloom add and the bright-pass half of `downsample.shader` are absent; every
// other line is the game's.
//
// This file is self-contained: it is compiled without `common.wgsl` ahead of it
// because the post stage reads no frame constants.

struct Post {
    /// `HSV` from `map/seasons.txt`: hue shift in sixths, saturation and value
    /// multipliers.
    hsv: vec4<f32>,
    /// `ColorBalance` from the same block.
    color_balance: vec4<f32>,
    /// `(MiddleGrey, hdr min, hdr max, night blend)`.
    grade: vec4<f32>,
    /// `(day LUT layer, night LUT layer, Levels min, Levels max)`.
    luts: vec4<f32>,
    /// `(luminance adaptation blend, unused, unused, unused)`.
    misc: vec4<f32>,
}

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var luminance: texture_2d<f32>;
@group(0) @binding(3) var previous_luminance: texture_2d<f32>;
@group(0) @binding(4) var color_cube: texture_2d_array<f32>;
@group(0) @binding(5) var linear_clamp: sampler;

const LUMINANCE_VECTOR: vec3<f32> = vec3<f32>(0.2125, 0.7154, 0.0721);
const CUBE_SIZE: f32 = 32.0;

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
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
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

/// `downsample.shader`'s log-luminance term. The bright-pass that shares the
/// pass in the game feeds bloom only, which this port does not draw.
@fragment
fn seed_main(input: FullScreen) -> @location(0) f32 {
    // A bilinear tap at a half-resolution texel centre is the mean of the four
    // scene texels under it, which is the `tex2Dlod0` the game takes.
    let color = textureSampleLevel(scene, linear_clamp, input.uv, 0.0).rgb;
    return log(max(0.0, dot(color, LUMINANCE_VECTOR)) + 0.0001);
}

/// One step of `downsample_luminance.shader`'s `LuminanceGather`: the mean of
/// the source level. The game gathers the whole level in one pass; a mip chain
/// of two-by-two means reaches the same average and costs a bounded loop.
@fragment
fn reduce_main(input: FullScreen) -> @location(0) f32 {
    let size = vec2<i32>(textureDimensions(luminance, 0));
    let base = vec2<i32>(input.position.xy) * 2;
    let last = size - vec2<i32>(1);
    var total = 0.0;
    for (var y = 0; y < 2; y = y + 1) {
        for (var x = 0; x < 2; x = x + 1) {
            let at = clamp(base + vec2<i32>(x, y), vec2<i32>(0), last);
            total = total + textureLoad(luminance, at, 0).r;
        }
    }
    return total * 0.25;
}

/// `clamp(exp(vSum), MinHdr, MaxHdr)` and `CalculateAdaptedLuminance`.
@fragment
fn adapt_main(input: FullScreen) -> @location(0) f32 {
    let mean = textureLoad(luminance, vec2<i32>(0, 0), 0).r;
    // HLSL `clamp` is `min(max(x, lo), hi)`, and the `green_night` volume ships
    // `hdr_min_adjustment 0.8` above `hdr_max_adjustment 0.6`, so the order
    // matters: written out rather than left to WGSL's `clamp`, which does not
    // define the inverted case.
    let current = min(max(exp(mean), post.grade.y), post.grade.z);
    let previous = textureLoad(previous_luminance, vec2<i32>(0, 0), 0).r;
    // `vLast + (vCurrent - vLast) * (1 - exp(-tau * dt))`, with the blend
    // supplied by the caller: 1 for a headless frame and a settled camera.
    return previous + (current - previous) * post.misc.x;
}

/// `SampleColorCube`: a 32-cubed LUT unwrapped along x into 1024x32, bilinear
/// within a slice and hand-blended across the two blue slices.
fn sample_color_cube(color: vec3<f32>, layer: i32) -> vec3<f32> {
    let scale = (CUBE_SIZE - 1.0) / CUBE_SIZE;
    let offset = 0.5 / CUBE_SIZE;

    let x = (scale * color.r + offset) / CUBE_SIZE;
    let y = scale * color.g + offset;

    let z_floor = floor((scale * color.b + offset) * CUBE_SIZE);
    let x_offset1 = z_floor / CUBE_SIZE;
    let x_offset2 = min(CUBE_SIZE - 1.0, z_floor + 1.0) / CUBE_SIZE;

    let color1 = textureSampleLevel(color_cube, linear_clamp, vec2<f32>(x + x_offset1, y), layer, 0.0).rgb;
    let color2 = textureSampleLevel(color_cube, linear_clamp, vec2<f32>(x + x_offset2, y), layer, 0.0).rgb;
    return mix(color1, color2, scale * color.b * CUBE_SIZE - z_floor);
}

/// `HuePost` (`standardfuncsgfx.fxh:91`), a hue in sixths rather than degrees.
fn hue_post(h: f32) -> vec3<f32> {
    let x = 1.0 - abs(h - 2.0 * floor(h * 0.5) - 1.0);
    if h < 1.0 { return vec3<f32>(1.0, x, 0.0); }
    if h < 2.0 { return vec3<f32>(x, 1.0, 0.0); }
    if h < 3.0 { return vec3<f32>(0.0, 1.0, x); }
    if h < 4.0 { return vec3<f32>(0.0, x, 1.0); }
    if h < 5.0 { return vec3<f32>(x, 0.0, 1.0); }
    return vec3<f32>(1.0, 0.0, x);
}

fn hsv_to_rgb_post(hsv: vec3<f32>) -> vec3<f32> {
    if hsv.y != 0.0 {
        let c = hsv.y * hsv.z;
        return clamp(hue_post(hsv.x) * c + (hsv.z - c), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return clamp(vec3<f32>(hsv.z), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn rgb_to_hsv(rgb: vec3<f32>) -> vec3<f32> {
    let c_max = max(rgb.r, max(rgb.g, rgb.b));
    let c_min = min(rgb.r, min(rgb.g, rgb.b));
    let diff = c_max - c_min;
    var h = 0.0;
    var s = 0.0;
    if diff != 0.0 {
        s = diff / c_max;
        if c_max == rgb.r {
            h = (rgb.g - rgb.b) / diff + 6.0;
        } else if c_max == rgb.g {
            h = (rgb.b - rgb.r) / diff + 2.0;
        } else {
            h = (rgb.r - rgb.g) / diff + 4.0;
        }
    }
    return vec3<f32>(h, s, c_max);
}

/// `Levels` (`standardfuncsgfx.fxh:172`).
fn levels(color: vec3<f32>, low: f32, high: f32) -> vec3<f32> {
    return clamp(clamp(color - low, vec3<f32>(0.0), vec3<f32>(1.0)) / (high - low), vec3<f32>(0.0), vec3<f32>(1.0));
}

/// `Uncharted2Tonemap` with the shader's own A..F.
fn uncharted2(x: vec3<f32>) -> vec3<f32> {
    let a = 0.22;
    let b = 0.30;
    let c = 0.10;
    let d = 0.20;
    let e = 0.01;
    let f = 0.30;
    return ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f;
}

@fragment
fn grade_main(input: FullScreen) -> @location(0) vec4<f32> {
    var color = textureSampleLevel(scene, linear_clamp, input.uv, 0.0).rgb;

    // `Exposure`, the `ADJUSTED_EXPOSURE` branch.
    let average = textureLoad(previous_luminance, vec2<i32>(0, 0), 0).r;
    color = color * (post.grade.x / average);

    // `ToneMap`, the `UNCHARTED` branch, including its `pow(1/2.2)`.
    let white_scale = vec3<f32>(1.0) / uncharted2(vec3<f32>(11.2));
    color = pow(uncharted2(color) * white_scale, vec3<f32>(1.0 / 2.2));
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));

    // `RestoreScene`, the `COLOR_LUT` branch. The engine cross-fades its LUTs
    // into one cube with `lut_blender.shader` before the frame; lerping the two
    // samples here is the same lerp one stage later.
    let day = sample_color_cube(color, i32(post.luts.x));
    let night = sample_color_cube(color, i32(post.luts.y));
    color = mix(day, night, post.grade.w);

    var hsv = rgb_to_hsv(color);
    hsv = vec3<f32>(hsv.x + post.hsv.x, hsv.y * post.hsv.y, hsv.z * post.hsv.z);
    hsv.x = hsv.x - 6.0 * floor(hsv.x / 6.0);
    color = hsv_to_rgb_post(hsv);

    color = clamp(color * post.color_balance.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    color = levels(color, post.luts.z, post.luts.w);
    return vec4<f32>(color, 1.0);
}
