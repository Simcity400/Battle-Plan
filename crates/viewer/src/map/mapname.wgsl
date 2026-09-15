// Port of `gfx/FX/mapname.shader`, the country and state name layer.
//
// The engine's `Transp_OffsetX` is `ConstantBuffer(1, 32)`, rebound once per
// name; this pass carries the same two floats per vertex so the whole layer is
// one draw (see `name_pass.rs`). Everything else is the shader as written.
//
// Verified against frame33840 PS 135334: compiling the retained mapname
// ps_5_0 cache with D3DCOMPILE_OPTIMIZATION_LEVEL3 reproduces all 3408 bytes
// (DXBC checksum DFA8F0D38DD19B71052792C25B3753AA). Its input signature has
// position/UV only: there is no country colour, font-definition colour, SDF
// threshold, shadow offset or procedural outline. See RENDERING.md's name
// material audit for the mip comparison and individual pixel histories.

// `DiffuseTexture`: the `hoi_mapfont4` atlas page. `MipMapLodBias = -0.4`,
// applied with `textureSampleBias` because wgpu's sampler has no bias field.
const MAPNAME_LOD_BIAS: f32 = -0.4;
// The 0.35 of `vSample.rgb *= 1.0f - ( DayNightFactor( ... ) * 0.35f )`.
const MAPNAME_NIGHT: f32 = 0.35;

@group(1) @binding(0) var name_page: texture_2d<f32>;
@group(1) @binding(1) var name_sampler: sampler;

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
    @location(2) alpha: f32,
    @location(3) offset_x: f32,
) -> NameOut {
    var out: NameOut;
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
