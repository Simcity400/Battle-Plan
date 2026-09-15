// particle.shader: VertexSimpleParticle / PixelSimpleParticle.
@group(1) @binding(0) var particle_texture: texture_2d<f32>;
@group(1) @binding(1) var particle_sampler: sampler;
struct ParticleOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) fog: f32,
}
@vertex fn particle_vertex(@builtin(vertex_index) index: u32,
    @location(0) pos_size: vec4<f32>, @location(1) rotation: vec4<f32>,
    @location(2) color: vec4<f32>, @location(3) fog: vec4<f32>) -> ParticleOut {
    let corners=array<vec2<f32>,6>(vec2(0.,0.),vec2(1.,0.),vec2(1.,1.),vec2(0.,0.),vec2(1.,1.),vec2(0.,1.));
    let uv=corners[index];
    let offset=(uv-0.5)*pos_size.w;
    let sy=sin(rotation.x); let cy=cos(rotation.x);
    let sp=sin(rotation.y); let cp=cos(rotation.y);
    let sr=sin(rotation.z); let cr=cos(rotation.z);
    let roll=vec2(offset.x*cr-offset.y*sr,offset.x*sr+offset.y*cr);
    var world=pos_size.xyz;
    if rotation.w==0.0 {
        let pitch=vec3(roll.x,roll.y*cp,roll.y*sp);
        world+=vec3(cy*pitch.x-sy*pitch.z,pitch.y,sy*pitch.x+cy*pitch.z);
    }
    var clip=globals.frame.view_projection*vec4(world,1.0);
    if rotation.w!=0.0 {clip=vec4(clip.xy+roll*vec2(globals.frame.view_projection[0][0],1.0/tan(0.785*0.5)),clip.zw);}
    var out:ParticleOut;
    out.position=clip; out.uv=vec2(uv.x,1.0-uv.y); out.world=world; out.color=color; out.fog=fog.x;
    return out;
}
@fragment fn particle_fragment(v:ParticleOut) -> @location(0) vec4<f32> {
    var color=textureSample(particle_texture,particle_sampler,v.uv)*v.color;
    if v.fog!=0.0 {color=vec4(apply_distance_fog(color.rgb,v.world),color.a);}
    return color;
}
