// Caster comparison port, retained until its isolated screenshot gate closes.
const TAU: f32 = 6.28318531;
const TREE_ALPHA_TEST: f32 = 0.5;
@group(1) @binding(0) var tree_diffuse: texture_2d<f32>;
@group(1) @binding(11) var linear_wrap: sampler;
@group(1) @binding(12) var linear_clamp: sampler;
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

/// `TreeMaskTexture`, sampler index 14 of `tree.shader`. `map::trees::tree_mask`
/// generates it from `map/rivers.bmp` the way the engine does, one byte per
/// texel at half the map's resolution.
@group(1) @binding(14) var tree_mask_texture: texture_2d<f32>;

struct TreePlacement {
    world: vec3<f32>,
    rand_sin: f32,
    rand_cos: f32,
    /// `vRandom`, the instance's rotation as a fraction of a turn.
    random: f32,
}

fn tree_placement(
    position: vec3<f32>,
    uv1: vec2<f32>,
    instance_position: vec3<f32>,
    instance_rotation: f32,
    instance_slope: vec2<f32>,
) -> TreePlacement {
    let random = instance_rotation / TAU;
    // `vSummedRandom = vUV1.x + vRandom; if >= 1 subtract 1`, which is `fract`
    // only because both terms are already inside the unit interval.
    var summed = uv1.x + random;
    if summed >= 1.0 {
        summed -= 1.0;
    }

    var p = position;
    p.y *= 0.75 + summed * 0.5;
    let rand_sin = sin(instance_rotation);
    let rand_cos = cos(instance_rotation);
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
    return TreePlacement(p, rand_sin, rand_cos, random);
}

fn tree_mask(world: vec3<f32>) -> f32 {
    let uv = vec2<f32>(
        (world.x - 0.5) / globals.map_size.x,
        (world.z + 2.0 - globals.map_size.y) / -globals.map_size.y,
    );
    return textureSample(tree_mask_texture, linear_clamp, wrap_u(uv)).a;
}

const TREE_SHADOW_MASK_CUTOFF: f32 = 0.1;

/// `VS_OUTPUT_SHADOW` (`tree.shader:178-184`).
struct TreeShadowVarying {
    @builtin(position) position: vec4<f32>,
    /// `Out.fDepth = Out.vPosition.zw`.
    @location(0) depth: vec2<f32>,
    @location(1) uv0: vec2<f32>,
    @location(2) world: vec3<f32>,
}

/// `VertexShaderShadow` (`tree.shader:262-295`). `ViewProjectionMatrix` is the
/// light's in this pass, because the engine swaps the frame constants rather
/// than the shader; `map::shadow` does the same.
@vertex
fn tree_shadow_vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    @location(5) instance_position: vec3<f32>,
    @location(6) instance_rotation: f32,
    @location(7) instance_slope: vec2<f32>,
) -> TreeShadowVarying {
    let placement = tree_placement(position, uv1, instance_position, instance_rotation, instance_slope);
    var out: TreeShadowVarying;
    out.world = placement.world;
    out.position = globals.frame.view_projection * vec4<f32>(placement.world, 1.0);
    out.depth = out.position.zw;
    out.uv0 = uv0;
    return out;
}

/// `PixelShaderShadow` (`tree.shader:412-421`), the `treeshadow` effect.
///
/// `float4( In.fDepth.xxx * In.fDepth.y, 1 )` is written as the game writes it.
/// The light's projection is orthographic, so `w` is 1 and the product is the
/// depth; `pdxmesh.shader` divides by `w` in the same place, and the two agree
/// only there. The alpha test has no camera-height condition here, unlike the
/// lit and unlit twins: a canopy casts a cut-out shadow at every zoom.
@fragment
fn tree_shadow_fragment(input: TreeShadowVarying) -> @location(0) vec4<f32> {
    let diffuse = textureSample(tree_diffuse, linear_wrap, input.uv0);
    if diffuse.a < TREE_ALPHA_TEST {
        discard;
    }
    if tree_mask(input.world) >= TREE_SHADOW_MASK_CUTOFF {
        discard;
    }
    return vec4<f32>(vec3<f32>(input.depth.x * input.depth.y), 1.0);
}
