// `gfx/FX/tree.shader`: the `tree` effect, ported literally.
//
// The tree layer is its own pipeline, not a `pdxmesh` one: R3 section 3.4
// records that the canopy meshes declare `PdxMeshAdvancedSnow` in their
// material and are nonetheless drawn through these two shaders. Everything the
// game's pixel shader does is ported below; each departure is commented.
//
// Dropped, with the reason:
//
// * `ShadowMap`, and its reason has been corrected. `graphics.shadows=no` is
//   not what gates the map's shadow pass — `mapRenderingOptions.draw_shadows`
//   is, the reference profile has it on, and below camera height 400 the engine
//   really does draw the `treeunlit` pre-pass this port lacks, so the buffer is
//   not the all-ones clear there and the canopy the game draws shadows itself.
//   Above 400 the term is exactly 1, because `ShadowFadeFactor` is 0 and the
//   saturate inside `GetShadowScaled` takes over. `fShadowTerm` is the literal
//   1.0 here, as it is in the terrain and river ports, and `terrain.wgsl`'s
//   header measures what that costs: nothing that can explain the scene's
//   brightness gap, since `GetShadowScaled(SHADOW_WEIGHT_TREE, ...)` cannot
//   scale the sun below 0.3 and the frame demands 0.17.
// * `CalculatePointLights`. Point lights come from combats and effects, which
//   no recorded table carries and which neither reference frame has
//   (R3 section 2.4).
// * `ApplyFOW`'s two factors, which the omniscient viewer leaves at zero.
//
// The `tree` effect declares no `Defines`, so `CalculateSunLight` takes the
// plain Phong branch (`standardfuncsgfx.fxh:642-663`), not the improved
// Blinn-Phong one the terrain uses.

/// `6.28318531f`, spelled as `tree.shader:205` spells it.
const TAU: f32 = 6.28318531;
/// `TREE_SPECULAR` (`constants.fxh:124`).
const TREE_SPECULAR: f32 = 0.1;
/// `TREE_ROUGHNESS` (`constants.fxh:125`).
const TREE_ROUGHNESS: f32 = 0.6;
/// `TREE_SEASON_MIN` (`constants.fxh:113`): the season tint only reaches world
/// `z / MAP_SIZE_Y` above this, i.e. the northern half of the map.
const TREE_SEASON_MIN: f32 = 0.5;
/// `TREE_SEASON_FADE_TWEAK` (`constants.fxh:114`).
const TREE_SEASON_FADE_TWEAK: f32 = 2.5;
/// `Tree_season.bmp` is eight columns wide, one per `tree_*` block of
/// `map/seasons.txt` (R3 section 3.4).
const TREE_SEASON_COLUMNS: f32 = 8.0;
/// `clip( 0.17 - GetTreeMask(...) )`: a tree is removed where the mask alpha
/// reaches this.
const TREE_MASK_CUTOFF: f32 = 0.17;
/// `if( vCamPos.y < 80.0f ) clip( vDiffuseColor.a - 0.5f )`: the diffuse alpha
/// is only tested when the camera is this low, which is why the canopy reads as
/// a solid wash from altitude rather than as foliage (R3 section 3.4).
const TREE_ALPHA_TEST_HEIGHT: f32 = 80.0;
/// The alpha threshold that test uses.
const TREE_ALPHA_TEST: f32 = 0.5;

@group(1) @binding(0) var tree_diffuse: texture_2d<f32>;
@group(1) @binding(1) var tree_normal: texture_2d<f32>;
/// `map/terrain/Tree_tint.bmp`, 16 x 16: one column of per-instance canopy
/// tints, picked by the instance's own rotation.
@group(1) @binding(2) var tint_map: texture_2d<f32>;
/// `map/terrain/Tree_season.bmp`, 8 x 16: one column per season.
@group(1) @binding(3) var season_map: texture_2d<f32>;
/// `ColorMap`, the terrain colormap.
@group(1) @binding(4) var color_map: texture_2d<f32>;
/// `ColorMapSecond`. The shipped content has one colormap, so the pass binds it
/// here too and `vSeasonLerp` blends a texture with itself; see `map::trees`.
@group(1) @binding(5) var color_map_second: texture_2d<f32>;
@group(1) @binding(6) var snow_mud_data: texture_2d<f32>;
@group(1) @binding(7) var gradient_border_channel1: texture_2d<f32>;
@group(1) @binding(8) var gradient_border_channel2: texture_2d<f32>;
@group(1) @binding(9) var province_secondary_color_map: texture_2d<f32>;
@group(1) @binding(10) var reflection_cube_map: texture_cube<f32>;
@group(1) @binding(11) var linear_wrap: sampler;
@group(1) @binding(12) var linear_clamp: sampler;

/// `ConstantBuffer( 1, 32 )` without `ShadowMapTextureMatrix`, which only the
/// dropped shadow term used, plus the world-wrap translation the viewer draws
/// its copies with (`map::border_pass::wrap_offsets`).
struct TreeConstants {
    world_offset_x: f32,
    /// `vTreeFade`, the layer's whole alpha.
    tree_fade: f32,
    /// `vSeasonLerp`.
    season_lerp: f32,
    /// `vSeasonColumn`, 0 to 7.
    season_column: f32,
}

@group(1) @binding(13) var<uniform> tree: TreeConstants;

struct TreeVarying {
    @builtin(position) position: vec4<f32>,
    /// `Out.vTexCoord0_TintUV.xy`.
    @location(0) uv0: vec2<f32>,
    /// `Out.vTexCoord0_TintUV.zw`.
    @location(1) tint_uv: vec2<f32>,
    /// `Out.vPos`, the instance's world position including the wrap offset.
    @location(2) world: vec3<f32>,
    @location(3) normal: vec3<f32>,
    @location(4) tangent: vec3<f32>,
    @location(5) bitangent: vec3<f32>,
    /// `Out.vPrePos_vSeasonColumn.w`.
    @location(6) season_u: f32,
}

@vertex
fn tree_vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    // Per instance: the 24-byte record `map::trees::TreeInstance`, which is the
    // engine's own (R3 section 3.2).
    @location(5) instance_position: vec3<f32>,
    @location(6) instance_rotation: f32,
    @location(7) instance_slope: vec2<f32>,
) -> TreeVarying {
    let rotation = instance_rotation;
    let random = rotation / TAU;
    // `vSummedRandom = vUV1.x + vRandom; if >= 1 subtract 1`, which is `fract`
    // only because both terms are already inside the unit interval.
    var summed = uv1.x + random;
    if summed >= 1.0 {
        summed -= 1.0;
    }

    var p = position;
    p.y *= 0.75 + summed * 0.5;
    let rand_sin = sin(rotation);
    let rand_cos = cos(rotation);
    p = vec3<f32>(
        p.x * rand_cos - p.z * rand_sin,
        p.y,
        p.x * rand_sin + p.z * rand_cos,
    );
    // The shear uses the already-rotated x and z and runs before the instance
    // translation, exactly as `tree.shader:218-220` orders it.
    p.y += p.x * instance_slope.x + p.z * instance_slope.y;
    p += vec3<f32>(
        instance_position.x + tree.world_offset_x,
        instance_position.y,
        instance_position.z,
    );

    var out: TreeVarying;
    out.world = p;
    // No world matrix: instances are already in world space.
    out.position = globals.frame.view_projection * vec4<f32>(p, 1.0);
    out.uv0 = uv0;
    out.normal = vec3<f32>(
        normal.x * rand_cos - normal.z * rand_sin,
        normal.y,
        normal.x * rand_sin + normal.z * rand_cos,
    );
    out.tangent = vec3<f32>(
        tangent.x * rand_cos - tangent.z * rand_sin,
        tangent.y,
        tangent.x * rand_sin + tangent.z * rand_cos,
    );
    out.bitangent = cross(out.tangent, out.normal) * tangent.w;
    out.tint_uv = uv1 + vec2<f32>(random, 0.0);
    out.season_u = tree.season_column / TREE_SEASON_COLUMNS + 1.0 / (2.0 * TREE_SEASON_COLUMNS);
    return out;
}

/// `GetTreeMask`: the map-sized mask whose alpha removes trees where it is set.
///
/// UNVERIFIED (R3 section 3.4): the binding. The texture comes from
/// `pdxmap + 0xc8` and the only map-sized texture with a meaningful alpha at
/// this uv is `colormap_rgb_cityemissivemask_a.dds`, whose alpha is the city
/// emissive mask — "no trees inside cities". Only 0.58 % of the 99 056
/// instances sit above the cutoff, so the reading is not measurable on either
/// reference frame. Note the uv offsets differ from the main one by half a
/// texel in x and by two units in z; both are the game's.
fn tree_mask(world: vec3<f32>) -> f32 {
    let uv = vec2<f32>(
        (world.x - 0.5) / MAP_SIZE_X,
        (world.z + 2.0 - MAP_SIZE_Y) / -MAP_SIZE_Y,
    );
    return textureSample(color_map, linear_wrap, wrap_u(uv)).a;
}

/// `ApplySnowTree`: a straight lerp to `SNOW_COLOR`, without the terrain's
/// normal-driven ridge term (the commented-out `vSnowFade` above it).
struct TreeSnow {
    color: vec3<f32>,
    alpha: f32,
}

fn apply_snow_tree(color: vec3<f32>, mud_snow: vec4<f32>) -> TreeSnow {
    let is_snow = get_snow(mud_snow);
    let snow_fade = saturate(is_snow * 1.5);
    var opacity = cam_distance(SNOW_CAM_MIN, SNOW_CAM_MAX);
    opacity = SNOW_OPACITY_MIN + opacity * (SNOW_OPACITY_MAX - SNOW_OPACITY_MIN);
    let alpha = snow_fade * opacity;
    return TreeSnow(mix(color, SNOW_COLOR, alpha), alpha);
}

@fragment
fn tree_fragment(input: TreeVarying) -> @location(0) vec4<f32> {
    let diffuse = textureSample(tree_diffuse, linear_wrap, input.uv0);
    if cam_pos().y < TREE_ALPHA_TEST_HEIGHT && diffuse.a < TREE_ALPHA_TEST {
        discard;
    }
    if tree_mask(input.world) >= TREE_MASK_CUTOFF {
        discard;
    }

    // The uv2 convention the terrain and river ports already use. `wrap_u`
    // sends a wrap copy back onto the map it was copied from, which is what
    // keeps the clamped gradient-border samplers from smearing at the seam.
    let uv = wrap_u(vec2<f32>(
        (input.world.x + 0.5) / MAP_SIZE_X,
        (input.world.z + 0.5 - MAP_SIZE_Y) / -MAP_SIZE_Y,
    ));

    var color = get_overlay(
        diffuse.rgb,
        textureSample(tint_map, linear_wrap, input.tint_uv).rgb,
        0.5,
    );
    let season_color_map = mix(
        textureSample(color_map, linear_wrap, uv),
        textureSample(color_map_second, linear_wrap, uv),
        tree.season_lerp,
    ).rgb;
    color = get_overlay(color, season_color_map, 0.25);

    let season_tree_fade = saturate(
        saturate(input.world.z / MAP_SIZE_Y - TREE_SEASON_MIN) * TREE_SEASON_FADE_TWEAK,
    );
    color += saturate(
        textureSample(season_map, linear_wrap, vec2<f32>(input.season_u, input.tint_uv.y)).rgb
            - vec3<f32>(0.5),
    ) * season_tree_fade;

    let normal_sample = normalize(
        textureSample(tree_normal, linear_wrap, input.uv0).rgb - vec3<f32>(0.5),
    );
    // `mul( vNormalSample, Create3x3( t, b, n ) )`: a row vector times a matrix
    // built from rows, i.e. the sum of those rows weighted by the components.
    // The game does not renormalise the result, so neither does this.
    var normal = normal_sample.x * normalize(input.tangent)
        + normal_sample.y * normalize(input.bitangent)
        + normal_sample.z * normalize(input.normal);

    let mud_snow = textureSample(snow_mud_data, linear_wrap, map_uv_bottom_up(input.world));
    let snow = apply_snow_tree(color, mud_snow);
    color = snow.color;

    let gradient = gradient_border_apply(
        color,
        uv,
        gradient_border_channel1,
        gradient_border_channel2,
        linear_clamp,
        1.0,
        globals.frame.gb_cam_dist_override_outline_cutoff.zw,
        globals.frame.gb_cam_dist_override_outline_cutoff.xy,
    );
    let secondary = secondary_color_mask(
        gradient.color,
        uv,
        province_secondary_color_map,
        linear_wrap,
        gradient.bloom_alpha,
    );
    color = secondary.color;
    let bloom_alpha = secondary.bloom_alpha;

    var properties: LightingProperties;
    properties.world_space_pos = input.world;
    properties.to_camera_dir = normalize(cam_pos() - input.world);
    properties.normal = normal;
    properties.diffuse = color;
    properties.glossiness = TREE_ROUGHNESS;
    properties.specular_color = vec3<f32>(TREE_SPECULAR);
    // `GetNonLinearGlossiness( vSpecular )`, on the specular rather than on the
    // glossiness (`tree.shader:406`). The Phong branch never reads it, so the
    // oddity does not reach a pixel; it is ported as written.
    properties.non_linear_glossiness = get_non_linear_glossiness(TREE_SPECULAR);

    let light = calculate_sun_light_phong(properties, 1.0, calculate_sun_direction(input.world));
    var diffuse_light = light.diffuse;

    // `texCUBEbias( EnvironmentMap, float4( reflect(...), 3 - vRoughness * 3 ) )`:
    // the fourth component is a mip bias, not a level.
    let eye_dir = normalize(input.world - cam_pos());
    let reflective_color = textureSampleBias(
        reflection_cube_map,
        linear_clamp,
        reflect(eye_dir, normal),
        3.0 - TREE_ROUGHNESS * 3.0,
    ).rgb;
    diffuse_light += reflective_color * TREE_SPECULAR;

    var out = compose_light_snow(properties, diffuse_light, light.specular, snow.alpha);

    let globe_normal = calc_globe_normal(input.world.xz);
    let fogged = apply_fow(out, 0.0, 0.0);
    out = mix(fogged, out, BORDER_FOW_REMOVAL_FACTOR * (1.0 - bloom_alpha));
    out = apply_distance_fog(out, input.world);
    // `DayNight` is `DayNightWithBlend` at a blend of 1
    // (`standardfuncsgfx.fxh:397-400`); the tree layer takes no border
    // desaturation override, unlike the terrain.
    out = day_night_with_blend(out, globe_normal, 1.0);

    return vec4<f32>(out, tree.tree_fade);
}
