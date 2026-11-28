struct Params {
    width: u32, height: u32, columns: u32, rows: u32,
    stride: u32, packed_ids: u32, count: u32, dispatch_stride: u32,
    opacity: u32, no_color: u32, distance_cap: u32, distance_in_b: u32,
}
struct Region { color: u32, alpha_offset: u32, alpha_count: u32, radius: u32, attributes: u32, fx: u32, fill: u32 }
struct Cell { bits: array<u32, 8>, distance: u32 }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> ids: array<u32>;
@group(0) @binding(2) var<storage, read> region_of: array<u32>;
@group(0) @binding(3) var<storage, read> regions: array<Region>;
@group(0) @binding(4) var<storage, read> alpha: array<u32>;
@group(0) @binding(5) var<storage, read_write> cells: array<Cell>;
@group(0) @binding(6) var<storage, read_write> distances: array<u32>;
@group(0) @binding(7) var<storage, read_write> output: array<u32>;
@group(0) @binding(8) var<storage, read> dirty: array<u32>;

fn region_at(x: u32, y: u32) -> u32 {
    let at = y * p.width + x;
    var id: u32;
    if p.packed_ids != 0u { id = (ids[at / 2u] >> ((at & 1u) * 16u)) & 65535u; }
    else { id = ids[at]; }
    return region_of[id];
}
fn boundary(x: u32, y: u32) -> bool {
    let here = region_at(x, y);
    if here == 0u { return false; }
    if region_at((x + p.width - 1u) % p.width, y) != here || region_at((x + 1u) % p.width, y) != here { return true; }
    if y > 0u && region_at(x, y - 1u) != here { return true; }
    return y + 1u < p.height && region_at(x, y + 1u) != here;
}
@compute @workgroup_size(8)
fn boundaries(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) word: u32) {
    let n = group.x + group.y * p.dispatch_stride;
    if n >= p.count { return; }
    let cell = dirty[n];
    let origin = vec2<u32>(cell % p.columns, cell / p.columns) * 16u;
    var bits = 0u;
    for (var bit = 0u; bit < 32u; bit++) {
        let index = word * 32u + bit;
        let x = origin.x + index % 16u;
        let y = origin.y + index / 16u;
        if x < p.width && y < p.height && boundary(x, y) { bits |= 1u << bit; }
    }
    cells[cell].bits[word] = bits;
}
@compute @workgroup_size(8, 8)
fn distance_init(@builtin(global_invocation_id) pos: vec3<u32>) {
    if pos.x >= p.columns || pos.y >= p.rows { return; }
    let cell = pos.y * p.columns + pos.x;
    var bits = 0u;
    for (var word = 0u; word < 8u; word++) { bits |= cells[cell].bits[word]; }
    cells[cell].distance = select(p.distance_cap, 0u, bits != 0u);
}
fn dilate(pos: vec2<u32>, from_b: bool) -> u32 {
    var best = p.distance_cap;
    for (var dy = -1; dy <= 1; dy++) {
        let y = i32(pos.y) + dy;
        if y < 0 || y >= i32(p.rows) { continue; }
        for (var dx = -1; dx <= 1; dx++) {
            let x = u32((i32(pos.x) + dx + i32(p.columns)) % i32(p.columns));
            let cell = u32(y) * p.columns + x;
            var d: u32;
            if from_b { d = distances[cell]; } else { d = cells[cell].distance; }
            best = min(best, d + select(1u, 0u, dx == 0 && dy == 0));
        }
    }
    return best;
}
@compute @workgroup_size(8, 8)
fn distance_to_b(@builtin(global_invocation_id) pos: vec3<u32>) {
    if pos.x < p.columns && pos.y < p.rows { distances[pos.y * p.columns + pos.x] = dilate(pos.xy, false); }
}
@compute @workgroup_size(8, 8)
fn distance_to_a(@builtin(global_invocation_id) pos: vec3<u32>) {
    if pos.x < p.columns && pos.y < p.rows { cells[pos.y * p.columns + pos.x].distance = dilate(pos.xy, true); }
}
fn distance_at(cell: u32) -> u32 {
    if p.distance_in_b != 0u { return distances[cell]; }
    return cells[cell].distance;
}
fn wrapped(a: u32, b: u32) -> u32 { let d = u32(abs(i32(a) - i32(b))); return min(d, p.width - d); }
fn bucket_distance(x: u32, y: u32, column: u32, row: u32) -> u32 {
    let x0 = column * 16u;
    let x1 = min((column + 1u) * 16u, p.width) - 1u;
    var dx = 0u;
    if x < x0 || x > x1 { dx = min(wrapped(x, x0), wrapped(x, x1)); }
    let y0 = row * 16u;
    let y1 = (row + 1u) * 16u - 1u;
    var dy = 0u;
    if y < y0 || y > y1 { dy = min(u32(abs(i32(y) - i32(y0))), u32(abs(i32(y) - i32(y1)))); }
    return dx * dx + dy * dy;
}
fn nearest(x: u32, y: u32, radius: u32) -> u32 {
    let cx = i32(x / 16u); let cy = i32(y / 16u); let r = i32(radius);
    if distance_at(u32(cy) * p.columns + u32(cx)) > radius { return 0xffffffffu; }
    var first = cx - r; var last = cx + r;
    if 2u * radius + 1u >= p.columns { first = 0; last = i32(p.columns) - 1; }
    var best = 0xffffffffu;
    for (var row = max(cy - r, 0); row <= min(cy + r, i32(p.rows) - 1); row++) {
        for (var col = first; col <= last; col++) {
            let column = u32((col + i32(p.columns)) % i32(p.columns));
            let cell = u32(row) * p.columns + column;
            if best != 0xffffffffu && bucket_distance(x, y, column, u32(row)) >= best { continue; }
            var stop = false;
            for (var word = 0u; word < 8u; word++) {
                var bits = cells[cell].bits[word];
                while bits != 0u {
                    let bit = firstTrailingBit(bits);
                    bits &= bits - 1u;
                    let index = word * 32u + bit;
                    let dx = wrapped(x, column * 16u + index % 16u);
                    let dy = u32(abs(i32(y) - (row * 16 + i32(index / 16u))));
                    let d = dx * dx + dy * dy;
                    if d < best { best = d; if best == 0u { return 0u; } }
                    else if best + 512u < d { stop = true; break; }
                }
                if stop { break; }
            }
        }
    }
    return best;
}
fn rgb(c: u32) -> vec3<u32> { return vec3<u32>(c & 255u, (c >> 8u) & 255u, (c >> 16u) & 255u); }
struct Sample { color: vec4<u32>, effects: vec4<u32> }
fn sample_pixel(x: u32, y: u32) -> Sample {
    let key = region_at(x, y);
    if key == 0u {
        if distance_at(y / 16u * p.columns + x / 16u) <= 1u {
            var sum = vec3<u32>(0u); var count = 0u;
            var fx = vec2<u32>(0u); var gradient = 0u; var has_gradient = false;
            var opacity = 0u; var pulse = 0u;
            for (var dy = -1; dy <= 1; dy++) {
                for (var dx = -1; dx <= 1; dx++) {
                    let nx = i32(x) + dx; let ny = i32(y) + dy;
                    if (dx == 0 && dy == 0) || nx < 0 || ny < 0 || nx >= i32(p.width) || ny >= i32(p.height) { continue; }
                    let neighbour = region_at(u32(nx), u32(ny));
                    if neighbour != 0u {
                        sum += rgb(regions[neighbour].color); count++;
                        let attr = regions[neighbour].attributes;
                        opacity += regions[neighbour].fx & 255u;
                        pulse += select(0u, 255u, (regions[neighbour].fx & 256u) != 0u);
                        fx = (fx + vec2<u32>(attr & 255u, (attr >> 8u) & 255u)) & vec2<u32>(255u);
                        if (attr & (1u << 24u)) != 0u { gradient += (attr >> 16u) & 255u; has_gradient = true; }
                    }
                }
            }
            if count != 0u { return Sample(vec4<u32>(sum / count, select(255u, gradient / count, has_gradient)), vec4<u32>(fx.x / count, opacity / count, pulse / count, fx.y / count)); }
        }
        return Sample(vec4<u32>(rgb(p.no_color), 255u), vec4<u32>(0u));
    }
    let region = regions[key];
    let squared = nearest(x, y, region.radius);
    var a = 0u;
    if squared < region.alpha_count {
        a = (alpha[region.alpha_offset + squared / 4u] >> ((squared & 3u) * 8u)) & 255u;
    }
    if (region.attributes & (1u << 24u)) != 0u { a = (region.attributes >> 16u) & 255u; }
    if a < 239u {
        let band = i32(f32(255u - a) * (1.0 - bitcast<f32>(region.fill)));
        a = u32(clamp(i32(a) - band, 0, 255));
    }
    return Sample(vec4<u32>(rgb(region.color), a), vec4<u32>(region.attributes & 255u, region.fx & 255u, select(0u, 255u, (region.fx & 256u) != 0u), (region.attributes >> 8u) & 255u));
}
fn pack(c: vec4<u32>) -> u32 { return c.x | (c.y << 8u) | (c.z << 16u) | (c.w << 24u); }
fn field_position(group: vec3<u32>, local: vec3<u32>) -> vec2<u32> {
    let n = group.x + group.y * p.dispatch_stride;
    if n >= p.count { return vec2<u32>(p.width, p.height); }
    let cell = dirty[n];
    return vec2<u32>(cell % p.columns, cell / p.columns) * 8u + local.xy;
}

fn field_sample(xy: vec2<u32>) -> Sample {
    var sum = vec4<u32>(0u); var effects = vec4<u32>(0u);
    for (var dx = 0u; dx < 2u; dx++) {
        let a = sample_pixel(xy.x * 2u + dx, xy.y * 2u);
        let b = sample_pixel(xy.x * 2u + dx, xy.y * 2u + 1u);
        sum += (a.color + b.color) / 2u;
        effects += (a.effects + b.effects) / 2u;
    }
    return Sample(sum / 2u, effects / 2u);
}

@compute @workgroup_size(8, 8)
fn field(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    let xy = field_position(group, local);
    if xy.x >= p.width / 2u || xy.y >= p.height / 2u { return; }
    let result = field_sample(xy);
    let at = xy.y * p.stride + xy.x;
    output[at] = pack(result.color);
    output[at + p.stride * (p.height / 2u)] = pack(result.effects);
}

@group(1) @binding(0) var colors_out: texture_storage_2d<rgba8unorm, write>;
@group(1) @binding(1) var effects_out: texture_storage_2d<rgba8unorm, write>;

fn store_field(group: vec3<u32>, local: vec3<u32>, top: bool, bottom: bool) {
    let xy = field_position(group, local);
    if xy.x >= p.width / 2u || xy.y >= p.height / 2u { return; }
    let result = field_sample(xy);
    let color = unpack4x8unorm(pack(result.color));
    let effects = unpack4x8unorm(pack(result.effects));
    if top {
        textureStore(colors_out, vec2<i32>(xy), color);
        textureStore(effects_out, vec2<i32>(xy), effects);
    }
    if bottom {
        let lower = vec2<i32>(xy + vec2<u32>(0u, textureDimensions(colors_out).y / 2u));
        textureStore(colors_out, lower, color);
        textureStore(effects_out, lower, effects);
    }
}

@compute @workgroup_size(8, 8)
fn field_top(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    store_field(group, local, true, false);
}

@compute @workgroup_size(8, 8)
fn field_bottom(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    store_field(group, local, false, true);
}

@compute @workgroup_size(8, 8)
fn field_both(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    store_field(group, local, true, true);
}

fn seed_stride() -> u32 {
    return (p.width / 2u + 3u) / 4u;
}

var<workgroup> seed_alpha: array<u32, 64>;

@compute @workgroup_size(8, 8)
fn field_seed(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    let xy = field_position(group, local);
    let inside = xy.x < p.width / 2u && xy.y < p.height / 2u;
    let lane = local.y * 8u + local.x;
    var alpha = 0u;
    if inside { alpha = field_sample(xy).color.a; }
    seed_alpha[lane] = alpha;
    workgroupBarrier();
    if inside && local.x % 4u == 0u {
        output[xy.y * seed_stride() + xy.x / 4u] = pack(vec4<u32>(
            seed_alpha[lane], seed_alpha[lane + 1u], seed_alpha[lane + 2u], seed_alpha[lane + 3u]));
    }
}

@compute @workgroup_size(8, 8)
fn restore_seed(@builtin(global_invocation_id) id: vec3<u32>) {
    let xy = id.xy;
    if xy.x >= p.width / 2u || xy.y >= p.height / 2u { return; }
    let word = output[xy.y * seed_stride() + xy.x / 4u];
    let alpha = (word >> ((xy.x % 4u) * 8u)) & 255u;
    textureStore(colors_out, vec2<i32>(xy), unpack4x8unorm(p.no_color | (alpha << 24u)));
    textureStore(effects_out, vec2<i32>(xy), vec4<f32>(0.0));
}
