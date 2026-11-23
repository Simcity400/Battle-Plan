struct Params {
    view_projection: mat4x4<f32>,
    look: vec4<f32>,
    map: vec4<u32>,
    surface: vec4<f32>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var heights: texture_2d<f32>;
@group(0) @binding(2) var unknown: texture_2d<f32>;
@group(1) @binding(0) var<storage, read> drawn_heights: array<u32>;

const SIDE: u32 = 64u;
const STORED_PER_BYTE: f32 = 10.0;

struct Hatch {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec2<f32>,
    @location(1) ground: f32,
}

fn height(at: vec2<u32>) -> f32 {
    let row = p.map.y - 1u - min(at.y, p.map.y - 1u);
    let stored = textureLoad(heights, vec2<u32>(at.x % p.map.x, row), 0).r * 255.0;
    return round(stored) * p.surface.z;
}

fn drawn_height(base: u32, corner: vec2<u32>) -> f32 {
    let pair = drawn_heights[base + corner.y * (SIDE + 1u) + corner.x];
    return f32(bitcast<i32>(pair << 16u) >> 16u) / STORED_PER_BYTE * p.surface.z;
}

fn unknown_at(world: vec2<f32>) -> bool {
    if any(world < vec2<f32>(0.0)) || any(world >= vec2<f32>(p.map.xy)) {
        return false;
    }
    let cell = min(vec2<u32>(world / p.surface.w), p.map.zw - 1u);
    return textureLoad(unknown, cell, 0).r > 0.0;
}

@vertex
fn hatch_vertex(
    @builtin(vertex_index) index: u32,
    @location(0) drawn: vec4<u32>,
    @location(1) base: u32,
) -> Hatch {
    var corner = vec2<u32>(index % (SIDE + 1u), index / (SIDE + 1u));
    if ((corner.x + corner.y) & 1u) == 1u {
        if (corner.y == 0u && (drawn.w & 1u) != 0u) || (corner.y == SIDE && (drawn.w & 2u) != 0u) {
            corner.x += 1u;
        } else if (corner.x == 0u && (drawn.w & 4u) != 0u) || (corner.x == SIDE && (drawn.w & 8u) != 0u) {
            corner.y += 1u;
        }
    }
    corner = min(corner, min((p.map.xy - drawn.xy) / drawn.z, vec2<u32>(SIDE)));
    let at = drawn.xy + corner * drawn.z;
    var world = vec3<f32>(f32(at.x), drawn_height(base, corner), f32(at.y));
    if p.surface.x > 0.5 {
        world.y = p.surface.y;
    }
    let clip = p.view_projection * vec4<f32>(world, 1.0);
    let pulled = p.view_projection * vec4<f32>(world - p.look.xyz * p.look.w, 1.0);
    var out: Hatch;
    out.position = vec4<f32>(clip.xy, pulled.z, clip.w);
    out.world = vec2<f32>(f32(at.x), f32(at.y));
    out.ground = height(at);
    return out;
}

@fragment
fn hatch_pixel(input: Hatch) -> @location(0) vec4<f32> {
    let s = input.world.x + input.world.y;
    let spacing = exp2(ceil(log2(max(fwidth(s), 0.0001) * 10.0)));
    let phase = fract(s / spacing);
    let wet = input.ground < p.surface.y;
    if wet != (p.surface.x > 0.5) || !unknown_at(input.world) {
        discard;
    }
    if phase < 0.25 {
        return vec4<f32>(0.04, 0.04, 0.05, 0.6);
    }
    if phase >= 0.5 && phase < 0.75 {
        return vec4<f32>(0.9, 0.9, 0.92, 0.5);
    }
    discard;
    return vec4<f32>(0.0);
}
