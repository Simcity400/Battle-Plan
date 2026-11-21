
const TILES_X: u32 = 16u;
const TILES_Y: u32 = 8u;
const TILES_AT: u32 = 11u;

@group(0) @binding(0) var now: texture_2d<f32>;
@group(0) @binding(1) var was: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> counts: array<atomic<u32>>;

@compute @workgroup_size(8, 8, 1)
fn compare(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(now);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let at = vec2<i32>(i32(id.x), i32(id.y));
    let difference = abs(textureLoad(now, at, 0) - textureLoad(was, at, 0));
    let largest = max(max(difference.r, difference.g), max(difference.b, difference.a)) * 255.0;
    if largest <= 0.0 {
        return;
    }
    let steps = u32(min(largest, 255.0) + 0.5);
    atomicAdd(&counts[0], 1u);
    if steps >= 1u {
        atomicAdd(&counts[1], 1u);
    }
    if steps >= 2u {
        atomicAdd(&counts[2], 1u);
    }
    if steps >= 8u {
        atomicAdd(&counts[3], 1u);
    }
    if steps >= 32u {
        atomicAdd(&counts[4], 1u);
    }
    atomicMax(&counts[5], steps);
    atomicAdd(&counts[6], steps);
    atomicMin(&counts[7], id.x);
    atomicMin(&counts[8], id.y);
    atomicMax(&counts[9], id.x);
    atomicMax(&counts[10], id.y);
    if steps >= 2u {
        let tile = min(id.y * TILES_Y / size.y, TILES_Y - 1u) * TILES_X
            + min(id.x * TILES_X / size.x, TILES_X - 1u);
        atomicAdd(&counts[TILES_AT + tile], 1u);
    }
}
