// `gfx/FX/border.shader` and `gfx/FX/strait.shader`, ported line for line.
//
// Both draw a flat ribbon over the terrain with the same depth trick: the clip-space z comes from a
// position pushed `vCamLookAtDir * 0.05` toward the camera, so the strip is depth-tested against
// the terrain but never intersects it (`border.shader` VS, `strait.shader` VS).
//
// Deviations from the game shaders, each with the line it replaces:
//
// * `world_offset_x` is added to the vertex before anything else. The engine draws three world-wrap
//   copies by translating the *camera* (R2 section 2.3, `param_7` is the 1/2/4 view mask of the
//   three copies at `view`, `view+0x230`, `view+0x450`); the border meshes are cut at the x seam, so
//   the viewer translates the geometry instead and gets the same picture.
// * Nothing on the shadow term: `GetShadowScaled( SHADOW_WEIGHT_BORDER, ... )` is ported and reads
//   the buffer `map::shadow` fills. The earlier reading here — the reference profile is
//   `shadows=no`, so there is nothing to sample — was wrong in its premise: what gates the map's
//   shadow pass is `mapRenderingOptions.draw_shadows`, which the reference profile has on, and
//   `graphics.shadows` reaches no function on the map path at all (`terrain.wgsl`'s header).
// * `MipMapLodBias = -0.5` is applied with `textureSampleBias` because wgpu's sampler has no lod
//   bias; `MipMapMaxLod = 4` is the sampler's `lod_max_clamp`.

/// `BORDER_TILE` (`constants.fxh`): the diffuse tiles once every `1 / 0.4` points along the strip.
const BORDER_TILE: f32 = 0.4;

/// `ConstantBuffer( 2, 48 )` from `border.shader`, with the wrap offset in the padding.
struct BorderConstants {
    /// `vSelectionColor`; always zero here, because the replay records no selection (R2 2.3).
    selection_color: vec4<f32>,
    /// `vTime_Transparency`: `x` is the game time the pulse runs on, `y` the camera-distance fade
    /// the drawing pass takes from `borders::class_alpha`.
    time_transparency: vec2<f32>,
    /// Viewer addition: world x translation of this world-wrap copy.
    world_offset_x: f32,
    pad0: f32,
    pad1: vec4<f32>,
}

@group(1) @binding(0) var border_diffuse: texture_2d<f32>;
@group(1) @binding(1) var border_sampler: sampler;
@group(1) @binding(2) var<uniform> border: BorderConstants;

struct BorderVarying {
    @builtin(position) position: vec4<f32>,
    /// `VS_OUTPUT_BORDER.pos`: the world position after the water clamp.
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

/// The z row of `ViewProjectionMatrix`, i.e. `GetMatrixData( ViewProjectionMatrix, 2, 0..3 )`.
///
/// `Camera::view_projection` is `[row][column]` in the row-vector convention and reaches WGSL as
/// four columns, so `view_projection * v` is the engine's `mul( ViewProjectionMatrix, v )` and the
/// z row is component 2 of each column.
fn clip_z_row() -> vec4<f32> {
    let m = globals.frame.view_projection;
    return vec4<f32>(m[0].z, m[1].z, m[2].z, m[3].z);
}

fn cam_look_at_dir() -> vec3<f32> {
    return globals.frame.cam_look_at_dir_global_time.xyz;
}

/// `SHADOW_WEIGHT_BORDER` (`constants.fxh:172`).
const SHADOW_WEIGHT_BORDER: f32 = 0.7;

@vertex
fn border_vertex(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<i32>,
) -> BorderVarying {
    var pos = vec4<f32>(position.x + border.world_offset_x, position.y, position.z, 1.0);

    // The mesh builder samples the terrain heightmap, which is below the sea at a coast, so a strip
    // over water is clamped up onto the water plane (R2 section 2.4).
    let clamp_height = saturate((WATER_HEIGHT - position.y) * 10000.0);
    pos.y = clamp_height * WATER_HEIGHT + (1.0 - clamp_height) * pos.y;

    var out: BorderVarying;
    out.pos = pos.xyz;

    let distorted = pos - vec4<f32>(cam_look_at_dir() * 0.05, 0.0);
    let clip = globals.frame.view_projection * pos;
    // Move z slightly closer to the camera to avoid intersections with terrain.
    let new_z = dot(distorted, clip_z_row());
    out.position = vec4<f32>(clip.xy, new_z, clip.w);
    out.uv = vec2<f32>(uv);
    return out;
}

@fragment
fn border_fragment(input: BorderVarying) -> @location(0) vec4<f32> {
    var color = textureSampleBias(
        border_diffuse,
        border_sampler,
        vec2<f32>(input.uv.y * BORDER_TILE, input.uv.x),
        -0.5,
    );

    let pulse = saturate(smoothstep(
        0.0,
        1.0,
        (0.8 - abs(input.uv.x - 0.5)) + sin(border.time_transparency.x * 2.5) * 0.15,
    )) * border.selection_color.a;
    color = vec4<f32>(
        saturate(color.rgb + border.selection_color.rgb * saturate(pulse - color.a * 0.35)),
        color.a,
    );

    // `vColor.rgb *= GetShadowScaled( SHADOW_WEIGHT_BORDER, Input.vScreenCoord, ShadowMap )`
    // (`border.shader:137-138`), read at this pixel out of the buffer `map::shadow` fills below
    // camera height 400.
    color = vec4<f32>(
        color.rgb * get_shadow_scaled(SHADOW_WEIGHT_BORDER, input.position),
        color.a,
    );
    let fow = apply_fow(color.rgb, 0.0, 0.0);
    var rgb = mix(fow, color.rgb, BORDER_FOW_REMOVAL_FACTOR);
    rgb = apply_distance_fog(rgb, input.pos) * max(1.0, pulse);
    rgb = day_night_with_blend(rgb, calc_globe_normal(input.pos.xz), 1.0);

    return vec4<f32>(rgb, max(color.a, pulse - 0.2) * border.time_transparency.y);
}

struct StraitVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) pos: vec3<f32>,
}

@vertex
fn strait_vertex(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
) -> StraitVarying {
    let pos = vec4<f32>(position.x + border.world_offset_x, position.y, position.z, 1.0);
    var out: StraitVarying;
    out.position = globals.frame.view_projection * pos;
    let distorted = pos - vec4<f32>(cam_look_at_dir() * 0.05, 0.0);
    out.position.z = dot(distorted, clip_z_row());
    out.uv = uv;
    out.pos = pos.xyz;
    return out;
}

/// `strait.shader`'s pixel shader: the texture alpha straight through, distance fog on the colour,
/// no lighting, no shadow, no fog of war (its two FoW samplers are declared but never read).
@fragment
fn strait_fragment(input: StraitVarying) -> @location(0) vec4<f32> {
    let color = textureSample(border_diffuse, border_sampler, input.uv);
    return vec4<f32>(apply_distance_fog(color.rgb, input.pos), color.a);
}
