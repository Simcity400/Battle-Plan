// `gfx/FX/pdxmesh.shader`: `VertexPdxMeshStandard` and `PixelPdxMeshStandard`,
// ported literally. The effect `Defines` that the pixel shader branches on are
// a per-material flag word here rather than a pipeline variant; `map::models`
// records why and lists the branches this port drops.
//
// The one structural departure is the world matrix. The engine hands each
// object a `float4x4 WorldMatrix` and then divides the position it feeds to
// lighting by `WorldMatrix[3][3]`, in which it stores the object's uniform
// scale (`pdxmesh.shader:265`, R3 section 2.2). This port has no world matrix:
// a map object's transform is a translation, a rotation about Y and a uniform
// scale, which the instance record carries directly and the mesh scale is baked
// into the vertex positions. `_WorldSpacePos` is therefore the object's true
// world position in map units, which is what R3 reads that division as
// producing; the division itself is UNVERIFIED and is not reproduced.

/// Flag bits of the per-material word, matching `models::Effect::bits`.
const FLAG_IMPROVED_BLINN_PHONG: u32 = 1u;
const FLAG_EMISSIVE: u32 = 2u;
const FLAG_SNOW: u32 = 4u;
const FLAG_ALPHA_TEST: u32 = 8u;

@group(1) @binding(0) var diffuse_map: texture_2d<f32>;
/// `SpecularMap`, which the engine calls the properties map under
/// `PDX_IMPROVED_BLINN_PHONG`.
@group(1) @binding(1) var specular_map: texture_2d<f32>;
@group(1) @binding(2) var normal_map: texture_2d<f32>;
@group(1) @binding(3) var snow_mud_data: texture_2d<f32>;
@group(1) @binding(4) var gradient_border_channel1: texture_2d<f32>;
@group(1) @binding(5) var gradient_border_channel2: texture_2d<f32>;
@group(1) @binding(6) var province_secondary_color_map: texture_2d<f32>;
@group(1) @binding(7) var environment_map: texture_cube<f32>;
@group(1) @binding(8) var linear_wrap: sampler;
@group(1) @binding(9) var linear_clamp: sampler;

/// `ConstantBuffer(1, 28)` reduced to what a static map object uses, plus the
/// viewer's own world-wrap offset and the layer alpha.
struct MeshSlot {
    world_offset_x: f32,
    alpha: f32,
    flags: u32,
    cubemap_intensity: f32,
}

@group(1) @binding(10) var<uniform> slot: MeshSlot;

/// `matBones`: the joint palette, three `vec4` rows per joint, every composed
/// entity's joints end to end. A vertex's joint slots index it directly.
///
/// The engine writes one entity's bones per draw; this pass composes the whole
/// layer's palette once at load, because a recorded hour has no animation clock
/// to advance and every instance of an entity is therefore in the same pose.
@group(1) @binding(11) var<storage, read> palette: array<vec4<f32>>;

fn has(flag: u32) -> bool {
    return (slot.flags & flag) != 0u;
}

/// One joint applied to one homogeneous vector: `w` of 1 transforms a point and
/// 0 a direction.
fn joint_apply(joint: u32, vector: vec4<f32>) -> vec3<f32> {
    let row = joint * 3u;
    return vec3<f32>(
        dot(palette[row], vector),
        dot(palette[row + 1u], vector),
        dot(palette[row + 2u], vector),
    );
}

/// Linear blend skinning of one vector, the engine's `PdxMesh*Skinned` vertex
/// path. A vertex of an unskinned mesh --- every sprawl house, every map
/// building, the map frame --- carries four zero weights and takes the early
/// return, so it reaches the rest of the shader as the exact bits
/// `map::models` baked.
fn mesh_skin(vector: vec4<f32>, joints: vec4<u32>, weights: vec4<f32>) -> vec3<f32> {
    if weights.x + weights.y + weights.z + weights.w <= 0.0 {
        return vector.xyz;
    }
    return weights.x * joint_apply(joints.x, vector)
        + weights.y * joint_apply(joints.y, vector)
        + weights.z * joint_apply(joints.z, vector)
        + weights.w * joint_apply(joints.w, vector);
}

/// `mul( WorldMatrix, vPosition )` for a map object: a rotation about +Y, the
/// only rotation `map/buildings.txt`, `map/unitstacks.txt` and the sprawl
/// generator carry, plus the uniform scale and the translation.
fn mesh_rotation(instance_rotation: f32, pitch: f32, roll: f32) -> mat3x3<f32> {
    let s = sin(instance_rotation);
    let c = cos(instance_rotation);
    return mat3x3<f32>(
        vec3<f32>(c, 0.0, s),
        vec3<f32>(0.0, 1.0, 0.0),
        vec3<f32>(-s, 0.0, c),
    ) * mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, cos(pitch), sin(pitch)), vec3<f32>(0.0, -sin(pitch), cos(pitch))) * mat3x3<f32>(vec3<f32>(cos(roll), sin(roll), 0.0), vec3<f32>(-sin(roll), cos(roll), 0.0), vec3<f32>(0.0, 0.0, 1.0));
}

fn mesh_world(
    rotate: mat3x3<f32>,
    position: vec3<f32>,
    instance_position: vec3<f32>,
    instance_scale: f32,
) -> vec3<f32> {
    return rotate * (position * instance_scale) + instance_position
        + vec3<f32>(slot.world_offset_x, 0.0, 0.0);
}

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) tangent: vec3<f32>,
    @location(2) bitangent: vec3<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) world: vec3<f32>,
    @location(5) @interpolate(flat) train_clip: vec4<f32>,
}

@vertex
fn mesh_vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) instance_position: vec3<f32>,
    @location(5) instance_rotation: f32,
    @location(6) instance_scale: f32,
    @location(7) joints: vec4<u32>,
    @location(8) weights: vec4<f32>,
    @location(9) instance_pitch: f32,
    @location(11) instance_roll: f32,
    @location(10) train_clip: vec4<f32>,
) -> MeshOut {
    let skinned_position = mesh_skin(vec4<f32>(position, 1.0), joints, weights);
    let skinned_normal = mesh_skin(vec4<f32>(normal, 0.0), joints, weights);
    let skinned_tangent = mesh_skin(vec4<f32>(tangent.xyz, 0.0), joints, weights);
    let rotate = mesh_rotation(instance_rotation, instance_pitch, instance_roll);
    var out: MeshOut;
    let world = mesh_world(rotate, skinned_position, instance_position, instance_scale);
    out.world = world;
    out.train_clip = train_clip;
    out.train_clip.x += slot.world_offset_x;
    out.clip = globals.frame.view_projection * vec4<f32>(world, 1.0);
    out.normal = normalize(rotate * skinned_normal);
    out.tangent = normalize(rotate * skinned_tangent);
    out.bitangent = normalize(cross(out.normal, out.tangent) * tangent.w);
    out.uv0 = uv0;
    return out;
}

/// `UnpackRRxGNormal` (`standardfuncsgfx.fxh:311-318`).
fn unpack_rrxg_normal(sample: vec4<f32>) -> vec3<f32> {
    let x = sample.g * 2.0 - 1.0;
    let y = -(sample.a * 2.0 - 1.0);
    let z = sqrt(saturate(1.0 - x * x - y * y));
    return vec3<f32>(x, y, z);
}

/// `UnpackNormal` (`standardfuncsgfx.fxh:303-308`).
fn unpack_normal(sample: vec3<f32>) -> vec3<f32> {
    var n = normalize(sample - 0.5);
    n.g = -n.g;
    return n;
}

/// `GetEnvmapMipLevel` (`standardfuncsgfx.fxh:562-565`).
fn envmap_mip_level(glossiness: f32) -> f32 {
    return (1.0 - glossiness) * 8.0;
}

/// `ApplySnowMesh` (`pdxmesh.shader`, the same body `railroad.shader` carries).
struct SnowMesh {
    color: vec3<f32>,
    alpha: f32,
}

fn apply_snow_mesh(color: vec3<f32>, normal: vec3<f32>, mud_snow: vec4<f32>) -> SnowMesh {
    let is_snow = get_snow(mud_snow) * 0.25;
    let fade = saturate(
        saturate(normal.y - saturate(1.0 - is_snow))
            * is_snow
            * 5.5
            * saturate((normal.y - 0.5) * 1000.0),
    );
    var opacity = cam_distance(SNOW_CAM_MIN, SNOW_CAM_MAX);
    opacity = SNOW_OPACITY_MIN + opacity * (SNOW_OPACITY_MAX - SNOW_OPACITY_MIN);
    var out: SnowMesh;
    out.color = mix(color, SNOW_COLOR, fade * opacity);
    out.alpha = saturate(is_snow * 1.5) * opacity;
    return out;
}

/// `NightAmbientPosX..NegZ` (`constants.fxh:32-37`): the *unit* ambient basis,
/// which is a different table from the terrain's `NightAmbientMap*`. With
/// `NIGHT_AMBIENT_BOOST = 3` every axis but `NegY` saturates to white, so a
/// non-snow mesh at full night is close to fullbright (R3 section 2.3).
const MESH_NIGHT_AMBIENT: array<vec3<f32>, 6> = array<vec3<f32>, 6>(
    vec3<f32>(2.0, 2.0, 2.0),
    vec3<f32>(0.2, 0.2, 0.2),
    vec3<f32>(0.1, 0.1, 0.1),
    vec3<f32>(0.0, 0.0, 0.0),
    vec3<f32>(3.0, 3.0, 3.0),
    vec3<f32>(0.8, 0.8, 0.8),
);

/// The day half of `ComposeLightMesh`'s basis is the constant buffer the
/// `AMBIENT_LIGHT_*` defines fill (`00_graphics.lua:1053-1058`), which the
/// frame block already carries.
fn mesh_day_ambient(index: u32) -> vec3<f32> {
    switch index {
        case 0u: { return globals.frame.ambient_pos_x_shadow_fade.rgb; }
        case 1u: { return globals.frame.ambient_neg_x_fow_fade.rgb; }
        case 2u: { return globals.frame.ambient_pos_y_min_mesh_alpha.rgb; }
        case 3u: { return globals.frame.ambient_neg_y_neg_fog.rgb; }
        case 4u: { return globals.frame.ambient_pos_z.rgb; }
        default: { return globals.frame.ambient_neg_z_cubemap.rgb; }
    }
}

/// `AmbientLight(WorldNormal, vDayFactor, DayAmbientColors, NightAmbientColors)`
/// with the unit basis (`standardfuncsgfx.fxh:456-472`).
fn mesh_ambient_light(world_normal: vec3<f32>, day_factor: f32) -> vec3<f32> {
    let squared = world_normal * world_normal;
    let ix = select(0u, 1u, world_normal.x < 0.0);
    let iy = select(2u, 3u, world_normal.y < 0.0);
    let iz = select(4u, 5u, world_normal.z < 0.0);
    let night = MESH_NIGHT_AMBIENT;
    return squared.x
        * mix(mesh_day_ambient(ix), saturate(NIGHT_AMBIENT_BOOST * night[ix]), day_factor)
        + squared.y
            * mix(mesh_day_ambient(iy), saturate(NIGHT_AMBIENT_BOOST * night[iy]), day_factor)
        + squared.z
            * mix(mesh_day_ambient(iz), saturate(NIGHT_AMBIENT_BOOST * night[iz]), day_factor);
}

/// `ComposeLightMesh` (`standardfuncsgfx.fxh:708-734`).
fn compose_light_mesh(
    properties: LightingProperties,
    diffuse_light: vec3<f32>,
    specular_light: vec3<f32>,
    snow_factor: f32,
) -> vec3<f32> {
    let day_night = day_night_factor(calc_globe_normal(properties.world_space_pos.xz));
    let ambient = mesh_ambient_light(properties.normal, day_night);
    let snow_ambient = calc_snow_ambient(snow_factor);
    return ((snow_ambient + ambient + diffuse_light) * properties.diffuse) * hdr_range()
        + specular_light;
}

@fragment
fn mesh_fragment(input: MeshOut) -> @location(0) vec4<f32> {
    let uv0 = input.uv0;
    let diffuse_sample = textureSample(diffuse_map, linear_wrap, uv0);
    if has(FLAG_ALPHA_TEST) && diffuse_sample.a - 1.0 < 0.0 {
        discard;
    }
    let pos = input.world;
    var color = diffuse_sample.rgb;
    let in_normal = normalize(input.normal);
    let properties_sample = textureSample(specular_map, linear_wrap, uv0);

    var lighting: LightingProperties;
    var normal_sample: vec3<f32>;
    var emissive = 0.0;
    if has(FLAG_IMPROVED_BLINN_PHONG) {
        let normal_map_sample = textureSample(normal_map, linear_wrap, uv0);
        emissive = normal_map_sample.b;
        normal_sample = unpack_rrxg_normal(normal_map_sample);
        lighting.glossiness = properties_sample.a;
    } else {
        emissive = properties_sample.b;
        normal_sample = unpack_normal(textureSample(normal_map, linear_wrap, uv0).rgb);
        lighting.specular_color = vec3<f32>(properties_sample.a);
        // `GLOSSINESS` is declared by no effect a map object uses, so the
        // plain branch takes `SPECULAR_WIDTH`.
        // `SPECULAR_WIDTH` and `MAP_SPECULAR_WIDTH` are both 15
        // (`constants.fxh:41-43`); common.wgsl carries the second name.
        lighting.glossiness = MAP_SPECULAR_WIDTH;
    }
    lighting.non_linear_glossiness = get_non_linear_glossiness(lighting.glossiness);

    let tbn = mat3x3<f32>(normalize(input.tangent), normalize(input.bitangent), in_normal);
    var normal = normalize(tbn * normal_sample);

    // `pdxmesh.shader:499` hard-codes this; the cascaded-shadow call beside it
    // is commented out in the game's own source (R3 section 2.4).
    let shadow_term = 1.0;

    var snow_alpha = 0.0;
    if has(FLAG_SNOW) {
        // `GetMudSnowColor( vPos, SnowMudData )`.
        let mud_snow = textureSample(snow_mud_data, linear_wrap, map_uv_bottom_up(pos));
        let snowed = apply_snow_mesh(color, normal, mud_snow);
        color = snowed.color;
        snow_alpha = snowed.alpha;
    }

    // `PDX_GRADIENT_BORDERS`: country borders and occupation stripes are
    // painted onto 3D objects with the same map uv the terrain uses
    // (R3 section 2.2). `wrap_u` sends a wrap copy back onto the map it was
    // copied from, as the tree and river ports do.
    let map_uv = wrap_u(vec2<f32>(
        (pos.x + 0.5) / MAP_SIZE_X,
        (pos.z + 0.5 - MAP_SIZE_Y) / -MAP_SIZE_Y,
    ));
    let border = gradient_border_apply(
        color,
        map_uv,
        gradient_border_channel1,
        gradient_border_channel2,
        linear_clamp,
        1.0,
        globals.frame.gb_cam_dist_override_outline_cutoff.zw,
        globals.frame.gb_cam_dist_override_outline_cutoff.xy,
    );
    let secondary = secondary_color_mask(
        border.color,
        map_uv,
        province_secondary_color_map,
        linear_wrap,
        border.bloom_alpha,
    );
    color = secondary.color;

    lighting.world_space_pos = pos;
    lighting.to_camera_dir = normalize(cam_pos() - pos);
    lighting.normal = normal;
    if has(FLAG_IMPROVED_BLINN_PHONG) {
        let spec_remapped = properties_sample.g * properties_sample.g * 0.4;
        let metalness_remapped = 1.0 - (1.0 - properties_sample.b) * (1.0 - properties_sample.b);
        lighting.diffuse = metalness_to_diffuse(metalness_remapped, color);
        lighting.specular_color = metalness_to_spec(metalness_remapped, color, spec_remapped);
    } else {
        lighting.diffuse = color;
    }

    let sun_direction = calculate_sun_direction(pos);
    var light: LightOut;
    if has(FLAG_IMPROVED_BLINN_PHONG) {
        light = calculate_sun_light(lighting, shadow_term, sun_direction);
    } else {
        light = calculate_sun_light_phong(lighting, shadow_term, sun_direction);
    }
    light = calculate_point_lights(lighting, light, has(FLAG_IMPROVED_BLINN_PHONG), globals.light_grid);
    var diffuse_light = light.diffuse;
    var specular_light = light.specular;

    if has(FLAG_IMPROVED_BLINN_PHONG) {
        let eye_dir = normalize(pos - cam_pos());
        let reflection = reflect(eye_dir, normal);
        let mip = envmap_mip_level(lighting.glossiness);
        let reflective = textureSampleLevel(environment_map, linear_clamp, reflection, mip).rgb
            * slot.cubemap_intensity;
        specular_light += reflective
            * fresnel_glossy(
                lighting.specular_color,
                -eye_dir,
                lighting.normal,
                lighting.glossiness,
            );
    }

    if has(FLAG_SNOW) {
        color = compose_light_snow(lighting, diffuse_light, specular_light, snow_alpha);
    } else {
        color = compose_light_mesh(lighting, diffuse_light, specular_light, snow_alpha);
    }

    let globe_normal = calc_globe_normal(pos.xz);
    var alpha = 0.0;
    if has(FLAG_EMISSIVE) {
        emissive *= day_night_factor(globe_normal);
        color = mix(color, diffuse_sample.rgb, emissive);
        alpha = emissive;
    }

    // `ApplyFOW( vColor, FogColorFactor, min( FogAlphaFactor, NegFogMultiplier ) )`
    // is the identity while both factors are zero, which is what an omniscient
    // viewer leaves them at, so it is not called.
    color = apply_distance_fog(color, pos);
    // `DayNight` is `DayNightWithBlend` at a blend of 1
    // (`standardfuncsgfx.fxh:397-400`).
    color = day_night_with_blend(color, globe_normal, 1.0);
    // The returned alpha is the emissive mask the engine's bloom reads, floored
    // by `MinMeshAlpha`; this pass writes colour only, so it never reaches a
    // target, but the value is kept as the shader computes it.
    // pdxmesh.shader TRAIN: white RGB, independent of diffuse alpha.
    // Zero-alpha cars outside the route are omitted by TrainPlan::field.
    if has(16u) {
        let delta = input.world.xz - input.train_clip.xy;
        let clip_alpha = select(0.0, 1.0, dot(delta, input.train_clip.zw) >= 0.0);
        return vec4<f32>(color, clip_alpha * smoothstep(0.0, 2.5, length(delta)) * slot.alpha);
    }
    return vec4<f32>(color, max(alpha, globals.frame.ambient_pos_y_min_mesh_alpha.w) * slot.alpha);
}

// ----------------------------------------------------------- the shadow effect
/// `VS_OUTPUT_PDXMESHSHADOW` (`pdxmesh.shader`), which carries the clip depth
/// and the uv the alpha-blend variant tests.
struct MeshShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) depth: vec2<f32>,
    @location(1) uv0: vec2<f32>,
}

/// `VertexPdxMeshStandardShadow` (`pdxmesh.shader:329-342`) and its
/// `*SkinnedShadow` twin (`:344-366`), which are the same shader up to the skin
/// and which this port merges the way it merges the lit pair: a vertex with
/// four zero weights takes `mesh_skin`'s early return, so one entry point
/// serves both. `ViewProjectionMatrix` is the light's here, because
/// `map::shadow` swaps the frame constants exactly as the engine does.
@vertex
fn mesh_shadow_vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) instance_position: vec3<f32>,
    @location(5) instance_rotation: f32,
    @location(6) instance_scale: f32,
    @location(7) joints: vec4<u32>,
    @location(8) weights: vec4<f32>,
    @location(9) instance_pitch: f32,
    @location(11) instance_roll: f32,
    @location(10) train_clip: vec4<f32>,
) -> MeshShadowOut {
    let world = mesh_world(
        mesh_rotation(instance_rotation, instance_pitch, instance_roll),
        mesh_skin(vec4<f32>(position, 1.0), joints, weights),
        instance_position,
        instance_scale,
    );
    var out: MeshShadowOut;
    out.clip = globals.frame.view_projection * vec4<f32>(world, 1.0);
    out.depth = out.clip.zw;
    out.uv0 = uv0;
    return out;
}

/// `PixelPdxMeshStandardShadow` (`pdxmesh.shader:661-668`) and
/// `PixelPdxMeshAlphaBlendShadow` (`:680-689`), which differ only in the alpha
/// test; the flag word picks between them the way it picks between the lit
/// variants.
///
/// `In.vDepthUV0.xxx / In.vDepthUV0.y` is `z / w`, where `tree.shader` writes
/// `z * w` in the same place. The two agree only for an orthographic light, and
/// that is the evidence `map::shadow` rests its projection on.
@fragment
fn mesh_shadow_fragment(input: MeshShadowOut) -> @location(0) vec4<f32> {
    if has(FLAG_ALPHA_TEST)
        && textureSample(diffuse_map, linear_wrap, input.uv0).a < 0.5 {
        discard;
    }
    return vec4<f32>(vec3<f32>(input.depth.x / input.depth.y), 1.0);
}
