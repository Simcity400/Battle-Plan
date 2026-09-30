// Retired tree caster arithmetic, isolated from the runtime port for input comparisons.
struct Constants {vp: mat4x4<f32>, map_size: vec4<f32>}
@group(0) @binding(0) var<uniform> globals: Constants;
@group(0) @binding(1) var diffuse: texture_2d<f32>;
@group(0) @binding(2) var mask: texture_2d<f32>;
@group(0) @binding(3) var wrap_sampler: sampler;
@group(0) @binding(4) var clamp_sampler: sampler;
struct Varying {
    @builtin(position) position:vec4<f32>,
    @location(0) depth:vec2<f32>,
    @location(1) uv:vec2<f32>,
    @location(2) world:vec3<f32>,
}
@vertex fn vertex_main(@location(0) position:vec3<f32>, @location(3) uv:vec2<f32>,
    @location(4) uv1:vec2<f32>, @location(5) pos_rot:vec4<f32>, @location(6) slopes:vec3<f32>) -> Varying {
    let random = pos_rot.w / 6.28318531;
    var summed = uv1.x + random;
    if summed >= 1.0 { summed -= 1.0; }
    var p = position;
    p.y *= 0.75 + summed * 0.5;
    let rand_sin = sin(pos_rot.w); let rand_cos = cos(pos_rot.w);
    p = vec3<f32>(p.x*rand_cos - p.z*rand_sin, p.y, p.x*rand_sin + p.z*rand_cos);
    p.y += p.x*slopes.x + p.z*slopes.y;
    p.y += slopes.z; // Native Float32x2 input supplies zero to the missing third lane.
    p += pos_rot.xyz;
    var out:Varying; out.world=p;out.position=globals.vp*vec4<f32>(p,1.);
    out.depth=out.position.zw;out.uv=uv;return out;
}
@fragment fn pixel_main(v:Varying)->@location(0) vec4<f32> {
    if textureSample(diffuse,wrap_sampler,v.uv).a < 0.5 {discard;}
    let uv=vec2<f32>((v.world.x-0.5)/globals.map_size.x,(v.world.z+2.-globals.map_size.y)/-globals.map_size.y);
    if textureSample(mask,clamp_sampler,uv).a > 0.1 {discard;}
    return vec4<f32>(vec3<f32>(v.depth.x*v.depth.y),1.);
}
