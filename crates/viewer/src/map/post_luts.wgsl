// Matching `lut_blender.shader`: point samples at the exact destination texel
// then one or two lerps, followed by the render target's native UNORM8 store.
@group(0) @binding(0) var<uniform> factors: vec4<f32>;
@group(0) @binding(1) var base_day: texture_2d<f32>;
@group(0) @binding(2) var base_night: texture_2d<f32>;
@group(0) @binding(3) var blend_day: texture_2d<f32>;
@group(0) @binding(4) var blend_night: texture_2d<f32>;

@vertex
fn vertex_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn blend_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(position.xy);
    let base = mix(textureLoad(base_day, at, 0).rgb, textureLoad(base_night, at, 0).rgb, factors.x);
    let blend = mix(textureLoad(blend_day, at, 0).rgb, textureLoad(blend_night, at, 0).rgb, factors.x);
    return vec4<f32>(mix(base, blend, factors.y), 1.0);
}
