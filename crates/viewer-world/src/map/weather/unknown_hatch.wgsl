// The viewer's own hatching over snow/mud cells the hour did not observe. It covers the terrain
// blocks the terrain pass drew, vertex for vertex at the heightmap's heights, and lies on the
// ground or on the water surface, depth-tested against what the world passes drew. The game's
// shaders keep reading the snow/mud texture unchanged.
struct Params {
    view_projection: mat4x4<f32>,
    // The camera's look direction, and how far the depth is pulled back along it.
    look: vec4<f32>,
    // The map's width and height in pixels, then the snow/mud grid's columns and rows.
    map: vec4<u32>,
    // Surface (0 ground, 1 water), the water surface's height, world units per height byte, map
    // pixels per cell.
    surface: vec4<f32>,
}
@group(0) @binding(0) var<uniform> p: Params;
// The heightmap the terrain's vertices stand on, rows top-down.
@group(0) @binding(1) var heights: texture_2d<f32>;
// One texel per snow/mud cell, rows counted up from the map's bottom, set where it is unknown.
@group(0) @binding(2) var unknown: texture_2d<f32>;

// Quads along a terrain block's side.
const SIDE: u32 = 64u;

struct Hatch {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec2<f32>,
    // The terrain's height under this point.
    @location(1) ground: f32,
}

// A vertex's height byte in world units; the map wraps in x and ends in z, as the terrain's do.
fn height(at: vec2<u32>) -> f32 {
    let row = p.map.y - 1u - min(at.y, p.map.y - 1u);
    let stored = textureLoad(heights, vec2<u32>(at.x % p.map.x, row), 0).r * 255.0;
    return round(stored) * p.surface.z;
}

fn unknown_at(world: vec2<f32>) -> bool {
    if any(world < vec2<f32>(0.0)) || any(world >= vec2<f32>(p.map.xy)) {
        return false;
    }
    let cell = min(vec2<u32>(world / p.surface.w), p.map.zw - 1u);
    return textureLoad(unknown, cell, 0).r > 0.5;
}

// One vertex of a block (origin, step, snapped edges) on the terrain's 65 by 65 vertex grid.
@vertex
fn hatch_vertex(@builtin(vertex_index) index: u32, @location(0) drawn: vec4<u32>) -> Hatch {
    var corner = vec2<u32>(index % (SIDE + 1u), index / (SIDE + 1u));
    // An odd vertex on an edge meeting a coarser block moves onto that block's next vertex.
    if ((corner.x + corner.y) & 1u) == 1u {
        if (corner.y == 0u && (drawn.w & 1u) != 0u) || (corner.y == SIDE && (drawn.w & 2u) != 0u) {
            corner.x += 1u;
        } else if (corner.x == 0u && (drawn.w & 4u) != 0u) || (corner.x == SIDE && (drawn.w & 8u) != 0u) {
            corner.y += 1u;
        }
    }
    // Quads past the map's edge, which the terrain does not draw, collapse onto it.
    corner = min(corner, min((p.map.xy - drawn.xy) / drawn.z, vec2<u32>(SIDE)));
    let at = drawn.xy + corner * drawn.z;
    let ground = height(at);
    var world = vec3<f32>(f32(at.x), ground, f32(at.y));
    if p.surface.x > 0.5 {
        world.y = p.surface.y;
    }
    let clip = p.view_projection * vec4<f32>(world, 1.0);
    // Only the depth is pulled toward the camera, so the hatch never fights the surface under it.
    let pulled = p.view_projection * vec4<f32>(world - p.look.xyz * p.look.w, 1.0);
    var out: Hatch;
    out.position = vec4<f32>(clip.xy, pulled.z, clip.w);
    out.world = vec2<f32>(f32(at.x), f32(at.y));
    out.ground = ground;
    return out;
}

@fragment
fn hatch_pixel(input: Hatch) -> @location(0) vec4<f32> {
    // Diagonal stripes fixed to the map, a power of two world units apart near ten pixels.
    let s = input.world.x + input.world.y;
    let spacing = exp2(ceil(log2(max(fwidth(s), 0.0001) * 10.0)));
    let phase = fract(s / spacing);
    // Ground shows above the water surface and water below it; each hatch keeps to its own, and
    // to unknown cells.
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
