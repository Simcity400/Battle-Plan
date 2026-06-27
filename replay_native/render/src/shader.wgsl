// Textured-quad shader for the M1 primitives (sprite / 9-slice / glyph).
//
// Vertices arrive in PIXEL space (top-left origin); the uniform `screen` carries
// the target size so we convert to clip space here. UVs are in 0..1 texel space.
// `tint` multiplies the sampled colour (used for glyph colour + sprite tinting).
// `mode` selects how the sampled texel becomes RGBA:
//   0 = straight RGBA texture (sprites, 9-slice): sample as-is.
//   1 = glyph: sample the font atlas, take the channel mask into RGB->coverage,
//       output tint.rgb with coverage*tint.a as alpha (so coloured text works on
//       a white/alpha glyph atlas).

struct Globals {
    screen: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) mode: u32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) @interpolate(flat) mode: u32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    // pixel -> NDC. y flips (pixel y down, NDC y up).
    let ndc = vec2<f32>(
        in.pos.x / globals.screen.x * 2.0 - 1.0,
        1.0 - in.pos.y / globals.screen.y * 2.0,
    );
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = in.uv;
    out.tint = in.tint;
    out.mode = in.mode;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(tex, samp, in.uv);
    if (in.mode == 1u) {
        // Glyph: the game font atlases store glyph coverage in the alpha channel
        // (chnl=15 / alphaChnl=1). Use alpha as coverage; emit straight-alpha
        // coloured text (the pipeline pre-multiplies in blend via the factors).
        let coverage = texel.a;
        return vec4<f32>(in.tint.rgb, coverage * in.tint.a);
    }
    // Straight-alpha sprite / 9-slice. Tint multiplies (tint=white => identity).
    return texel * in.tint;
}
