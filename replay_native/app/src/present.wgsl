// PRESENT pass: sample the offscreen RGBA8 compositor target and write it into the
// surface frame, doing the colour-space conversion the surface format needs.
//
// The offscreen target is `Rgba8Unorm` holding straight, DISPLAY-space byte values
// (the same the golden-PNG path stores). If the surface is an *_Srgb format, the GPU
// applies a linear->srgb encode on write, so we must pass LINEAR values to the target
// to land the same display bytes. `srgb_out == 1` => decode (srgb->linear) here so the
// hardware re-encode round-trips to the original display values; `srgb_out == 0` =>
// pass through unchanged (Codex §1 sRGB trap).

struct PresentParams {
    // 1.0 if the surface is an sRGB format (decode here), else 0.0.
    // Three trailing scalar pads keep the struct a flat 16 bytes (matching the Rust
    // `PresentParams` exactly); a `vec3<f32>` would force 16-byte member alignment and
    // make the struct 32 bytes, mismatching the 16-byte uniform buffer.
    srgb_out: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_smp: sampler;
@group(1) @binding(0) var<uniform> params: PresentParams;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// A full-screen triangle generated from the vertex index (no vertex buffer).
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var out: VsOut;
    // (-1,-1), (3,-1), (-1,3) covers the screen; uv derived from clip pos.
    let x = f32((vid << 1u) & 2u);   // 0,2,0
    let y = f32(vid & 2u);           // 0,0,2
    let ndc = vec2<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0);
    out.pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0); // flip Y so uv.y=0 is the top
    out.uv = vec2<f32>(x, y);
    return out;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    let cutoff = step(vec3<f32>(0.04045), c);
    return mix(lo, hi, cutoff);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(src_tex, src_smp, in.uv);
    var rgb = texel.rgb;
    if (params.srgb_out > 0.5) {
        // The target re-encodes linear->srgb on write; feed it the linear form of our
        // display bytes so the round-trip reproduces them exactly.
        rgb = srgb_to_linear(rgb);
    }
    return vec4<f32>(rgb, 1.0);
}
