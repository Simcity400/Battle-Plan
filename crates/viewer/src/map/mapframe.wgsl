// `gfx/FX/pdxmesh.shader`: the `PdxMeshBorder` effect, ported literally.
//
// This is the map's table frame — the stone band the map sits on, beyond its
// top and bottom edges, with `the game` embossed on the top band. It
// is three `PdxMesh` ambient objects, not a screen-space sprite, so it goes
// through the ordinary mesh vertex shader and this pixel shader.
//
// The effect is `VertexShader = "VertexPdxMeshStandard"`, `PixelShader =
// "PixelPdxMeshBorder"`, `Defines = { "ALPHA_TEST" }`.
//
// What makes it unlike every other map pass, and what this port keeps:
//
// * Its own **fixed** sun. `ImprovedBlinnPhong(BORDER_SUN_INTENSITY,
//   normalize(BORDER_SUN_DIRECTION), ...)` takes the two `constants.fxh`
//   constants under the file's own "MAP BORDER (PDXMESH)" heading, not
//   `vVirtualSunPos` or `DayNight_Hour_SunDir`. The frame's lighting does not
//   move with the hour.
// * **Day ambient only.** `AmbientLight( normal, 0.0, DayAmbientColors,
//   DayAmbientColors )` passes the day cube for both cubes and a day factor of
//   zero, so the night cube and `NIGHT_AMBIENT_BOOST` never reach it. The
//   terminator does not cross the frame.
// * **No distance fog.** `ApplyDistanceFog` is commented out in the shipped
//   shader, alone among the map meshes.
// * **No cube reflection.** The `EnvironmentMap` block is commented out too.
// * **Alpha 0 out.** `return float4( vColor, 0 )`: the target's alpha is the
//   city-light bloom mask the terrain pass writes, and the frame contributes
//   nothing to it.
//
// Dropped, with the reason:
//
// * The shadow pass. `PdxMeshBorderShadow` maps to `PixelPdxMeshNoShadow`,
//   which is `clip(-1)`, so the frame casts no shadow even when shadows are on;
//   the reference profile has `shadows=no` anyway (R1 section 0).

/// `BORDER_SUN_INTENSITY` (`constants.fxh:246`).
const BORDER_SUN_INTENSITY: vec3<f32> = vec3<f32>(1.5, 1.5, 1.6);
/// `BORDER_SUN_DIRECTION` (`constants.fxh:247`), normalised at use as the
/// shader normalises it.
const BORDER_SUN_DIRECTION: vec3<f32> = vec3<f32>(-0.2, 0.9, 0.1);

/// Three scalars pad the slot rather than a `vec3<f32>`, which the uniform
/// address space would align to 16 and so grow the slot to 32 bytes.
struct FrameConstants {
    /// The world-wrap copy's x translation, as every other map pass takes it.
    world_offset_x: f32,
    padding_0: f32,
    padding_1: f32,
    padding_2: f32,
}

@group(1) @binding(0) var diffuse_map: texture_2d<f32>;
@group(1) @binding(1) var normal_map: texture_2d<f32>;
@group(1) @binding(2) var specular_map: texture_2d<f32>;
@group(1) @binding(3) var mesh_sampler: sampler;
@group(1) @binding(4) var<uniform> frame_constants: FrameConstants;

struct MapFrameVertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
}

struct MapFrameVertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec3<f32>,
    @location(3) bitangent: vec3<f32>,
    @location(4) uv0: vec2<f32>,
}

// `VertexPdxMeshStandard`: world matrix, then `ViewProjectionMatrix`. The
// ambient object's world matrix is a uniform scale and a translation (every
// frame placement in `map/ambient_object.txt` has a zero rotation), so it is
// folded into the vertex buffer on the CPU and only the wrap offset is left
// here. A uniform scale leaves the normal and tangent directions alone, so the
// shipped `mul( (float3x3)WorldMatrix, ... )` is the identity for them.
@vertex
fn map_frame_vertex(input: MapFrameVertexIn) -> MapFrameVertexOut {
    var out: MapFrameVertexOut;
    let world = input.position + vec3<f32>(frame_constants.world_offset_x, 0.0, 0.0);
    out.clip_position = globals.frame.view_projection * vec4<f32>(world, 1.0);
    out.world_position = world;
    out.normal = input.normal;
    out.tangent = input.tangent.xyz;
    // `CalcBitangent`: the cross of the normal and the tangent, signed by the
    // handedness the `.mesh` stores in the tangent's w.
    out.bitangent = cross(input.normal, input.tangent.xyz) * input.tangent.w;
    out.uv0 = input.uv0;
    return out;
}

/// `UnpackRRxGNormal` (`standardfuncsgfx.fxh:311-318`): the normal map is
/// stored with x in green and y in alpha, y flipped, z reconstructed.
fn unpack_rrxg_normal(sample: vec4<f32>) -> vec3<f32> {
    let x = sample.g * 2.0 - 1.0;
    let y = -(sample.a * 2.0 - 1.0);
    return vec3<f32>(x, y, sqrt(saturate(1.0 - x * x - y * y)));
}

@fragment
fn map_frame_fragment(input: MapFrameVertexOut) -> @location(0) vec4<f32> {
    let diffuse_sample = textureSample(diffuse_map, mesh_sampler, input.uv0);
    // `ALPHA_TEST`: `clip( vDiffuse.a - 1.0 )` keeps only fully opaque texels.
    if diffuse_sample.a - 1.0 < 0.0 {
        discard;
    }
    let properties = textureSample(specular_map, mesh_sampler, input.uv0);
    let normal_sample = unpack_rrxg_normal(textureSample(normal_map, mesh_sampler, input.uv0));

    let in_normal = normalize(input.normal);
    // `Create3x3( tangent, bitangent, normal )` builds the rows of the TBN, and
    // `mul( vNormalSample, TBN )` is a row-vector product, so the world normal
    // is the sample's components against those three rows.
    let tangent = normalize(input.tangent);
    let bitangent = normalize(input.bitangent);
    let world_normal = normalize(
        normal_sample.x * tangent + normal_sample.y * bitangent + normal_sample.z * in_normal,
    );

    let spec_remapped = properties.g * properties.g * 0.4;
    let metalness_remapped = 1.0 - (1.0 - properties.b) * (1.0 - properties.b);
    let glossiness = properties.a;
    var lighting: LightingProperties;
    lighting.world_space_pos = input.world_position;
    lighting.to_camera_dir = normalize(cam_pos() - input.world_position);
    lighting.normal = world_normal;
    lighting.diffuse = metalness_to_diffuse(metalness_remapped, diffuse_sample.rgb);
    lighting.specular_color =
        metalness_to_spec(metalness_remapped, diffuse_sample.rgb, spec_remapped);
    lighting.glossiness = glossiness;
    lighting.non_linear_glossiness = get_non_linear_glossiness(glossiness);

    let light = improved_blinn_phong(
        BORDER_SUN_INTENSITY,
        normalize(BORDER_SUN_DIRECTION),
        lighting,
    );
    // The night cube is never reached: the shipped call passes the day cube
    // twice with a day factor of zero.
    let ambient = ambient_light(lighting.normal, 0.0);
    let diffuse = ((ambient + light.diffuse) * lighting.diffuse) * hdr_range();
    return vec4<f32>(diffuse + light.specular, 0.0);
}
