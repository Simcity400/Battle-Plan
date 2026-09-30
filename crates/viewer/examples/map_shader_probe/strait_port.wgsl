// Isolated original strait kernel and its distance-fog helper.
struct Constants { values: array<vec4<f32>,26>, }
@group(0) @binding(0) var<uniform> cb: Constants;
@group(0) @binding(1) var diffuse: texture_2d<f32>;
@group(0) @binding(2) var diffuse_sampler: sampler;
struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) pos: vec3<f32>,
}
@vertex fn vertex_main(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> Out {
    let matrix=mat4x4<f32>(cb.values[0],cb.values[1],cb.values[2],cb.values[3]);
    let pos=vec4<f32>(position,1.0);
    var out: Out;
    out.clip=matrix*pos;
    let distorted=pos-vec4<f32>(cb.values[14].xyz*0.05,0.0);
    out.clip.z=dot(distorted,vec4<f32>(matrix[0].z,matrix[1].z,matrix[2].z,matrix[3].z));
    out.uv=uv;
    out.pos=pos.xyz;
    return out;
}
@fragment fn fragment_main(input: Out) -> @location(0) vec4<f32> {
    let color=textureSample(diffuse,diffuse_sampler,input.uv);
    let difference=cb.values[13].xyz-input.pos;
    let fog_factor=1.0-abs(normalize(difference).y);
    let square_distance=dot(difference,difference);
    let fog=clamp(min((square_distance-1.0)/(150.0*150.0-1.0),0.35),0.0,1.0)*fog_factor;
    return vec4<f32>(mix(color.rgb,vec3<f32>(0.12,0.28,0.6),fog),color.a);
}
