struct Camera {
    center: vec2<f32>,
    span: vec2<f32>,
    viewport: vec2<f32>,
    map_size: vec2<f32>,
};

@group(0) @binding(0) var provinces: texture_2d<u32>;
@group(0) @binding(1) var colors: texture_1d<f32>;
@group(0) @binding(2) var owners: texture_1d<u32>;
@group(0) @binding(3) var<uniform> camera: Camera;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[index], 0.0, 1.0);
}

fn province_at(position: vec2<f32>) -> u32 {
    let x = position.x - floor(position.x / camera.map_size.x) * camera.map_size.x;
    let y = clamp(position.y, 0.0, camera.map_size.y - 1.0);
    return textureLoad(provinces, vec2<i32>(i32(x), i32(y)), 0).r;
}

fn owner_at(position: vec2<f32>) -> u32 {
    return textureLoad(owners, i32(province_at(position)), 0).r;
}

@fragment
fn fragment(@builtin(position) screen: vec4<f32>) -> @location(0) vec4<f32> {
    let position = camera.center + (screen.xy / camera.viewport - vec2<f32>(0.5)) * camera.span;
    if position.y < 0.0 || position.y >= camera.map_size.y {
        return vec4<f32>(0.055, 0.065, 0.080, 1.0);
    }
    let id = province_at(position);
    let color = textureLoad(colors, i32(id), 0);
    let owner = textureLoad(owners, i32(id), 0).r;
    // Keep country borders readable as the map zooms out. No province values are interpolated.
    let radius = max(1.0, camera.span.y / camera.viewport.y * 0.7);
    let border = owner_at(position + vec2<f32>(radius, 0.0)) != owner
        || owner_at(position - vec2<f32>(radius, 0.0)) != owner
        || owner_at(position + vec2<f32>(0.0, radius)) != owner
        || owner_at(position - vec2<f32>(0.0, radius)) != owner;
    return vec4<f32>(color.rgb * select(1.0, 0.48, border), color.a);
}
