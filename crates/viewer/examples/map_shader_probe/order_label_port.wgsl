// Comparison port retained for order labels until their native-input gate closes.
// Country names always execute the compiled content shader.
//
// The engine's `Transp_OffsetX` is `ConstantBuffer(1, 32)`, rebound once per
// name. This pass keeps the glyphs static and reads the same two floats from
// `copies` by the vertex's name index and the instance (the copy) being drawn
// (see `name_pass.rs`). Everything else is the shader as written.
//
// Verified against frame33840 PS 135334: compiling the retained mapname
// ps_5_0 cache with D3DCOMPILE_OPTIMIZATION_LEVEL3 reproduces all 3408 bytes
// (DXBC checksum DFA8F0D38DD19B71052792C25B3753AA). Its input signature has
// position/UV only: there is no country colour, font-definition colour, SDF
// threshold, shadow offset or procedural outline. See [archived rendering notes](#)'s name
// material audit for the mip comparison and individual pixel histories.

// `DiffuseTexture`: the `hoi_mapfont4` atlas page. `MipMapLodBias = -0.4`,
// applied with `textureSampleBias` because wgpu's sampler has no bias field.
const MAPNAME_LOD_BIAS: f32 = -0.4;
// The 0.35 of `vSample.rgb *= 1.0f - ( DayNightFactor( ... ) * 0.35f )`.
const MAPNAME_NIGHT: f32 = 0.35;

@group(1) @binding(0) var name_page: texture_2d<f32>;
@group(1) @binding(1) var name_sampler: sampler;

// Per frame: `Transp_OffsetX.y` of each copy, then `stride` values of
// `Transp_OffsetX.x` per name, zero for a copy the frame does not draw.
struct NameCopies {
    offsets: array<vec4<f32>, 2>,
    stride: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    alpha: array<f32>,
}
@group(2) @binding(0) var<storage, read> copies: NameCopies;

struct NameOut {
    @builtin(position) clip: vec4<f32>,
    // `Out.vPrepos = v.vPosition.xyz`: the unshifted world position.
    @location(0) prepos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) alpha: f32,
}

@vertex
fn vertex_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) name: u32,
    @builtin(instance_index) copy: u32,
) -> NameOut {
    var out: NameOut;
    // `Transp_OffsetX.x`: a copy the per-name loop would have skipped moves
    // every vertex to one point outside the view, so it rasterises nothing.
    let alpha = copies.alpha[name * copies.stride + copy];
    let offset_x = copies.offsets[copy / 4u][copy % 4u];
    if (alpha <= 0.0) {
        out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.prepos = position;
        out.uv = uv;
        out.alpha = 0.0;
        return out;
    }
    // `vPos.x += Transp_OffsetX.y;` — the world-wrapped copy.
    let shifted = vec4<f32>(position.x + offset_x, position.y, position.z, 1.0);
    // "move z value slightly closer to camera to avoid intersections with
    // terrain": the clip z is taken from a position pulled half a world unit
    // along the camera's forward axis, while xy and w come from the real one.
    let distorted = shifted - vec4<f32>(globals.frame.cam_look_at_dir_global_time.xyz * 0.5, 0.0);
    let clip = globals.frame.view_projection * shifted;
    // `dot( vDistortedPos, row 2 of ViewProjectionMatrix )` is the z component
    // of the same transform, which is what this multiply takes.
    let biased_z = (globals.frame.view_projection * distorted).z;
    out.clip = vec4<f32>(clip.xy, biased_z, clip.w);
    out.prepos = position;
    out.uv = uv;
    out.alpha = alpha;
    return out;
}

@fragment
fn fragment_main(in: NameOut) -> @location(0) vec4<f32> {
    var sample = textureSampleBias(name_page, name_sampler, in.uv, MAPNAME_LOD_BIAS);
    sample.a = sample.a * in.alpha;
    // The authored body is black at alpha 128/255, with a pale translucent
    // rim (e.g. white at alpha 70/255). Both RGB and alpha must be filtered
    // as straight UNORM, using the authored mip chain. Night scales RGB
    // alone; the tint seen on screen also contains the background mixed by
    // SrcAlpha/InvSrcAlpha and the subsequent HDR scene grade. Neither the
    // core's black nor its alpha should be replaced by an opaque white fill.
    let night = day_night_factor(calc_globe_normal(in.prepos.xz));
    return vec4<f32>(sample.rgb * (1.0 - night * MAPNAME_NIGHT), sample.a);
}
