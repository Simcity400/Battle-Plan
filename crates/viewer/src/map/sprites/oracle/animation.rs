//! Independent test oracle for sprite_animation.fxh; never part of runtime rendering.
pub fn animated_source(records: &[[[f32; 4]; 5]], apply: bool) -> String {
    let mut source = super::EXACT_SHADER.to_string();
    // Keep vertex interpolation in the same stage as the native program.
    source = source.replace("    @location(0) uv: vec2<f32>,", "    @location(9) animated0: vec2<f32>,\n    @location(10) animated1: vec2<f32>,\n    @location(0) uv: vec2<f32>,");
    source = source.replace("    return out;", "    out.animated0 = animation_uv(uv, 0u);\n    out.animated1 = animation_uv(uv, 1u);\n    return out;");
    source.push_str(r#"
@group(1) @binding(0) var<uniform> data:array<vec4<f32>,10>;
@group(0) @binding(6) var mask_sampler:sampler;
@group(0) @binding(7) var animation_sampler:sampler;
@group(0) @binding(3) var animated_texture0: texture_2d<f32>;
@group(0) @binding(4) var mask_texture1: texture_2d<f32>;
@group(0) @binding(5) var animated_texture1: texture_2d<f32>;
fn animation_uv(uv:vec2<f32>, n:u32) -> vec2<f32> {
    let r = data[n*5u]; let scale = data[n*5u+1u];
    let placement = data[n*5u+2u]; let motion = data[n*5u+3u];
    let p = (placement.xy + uv - vec2<f32>(0.5)) * scale.xy * placement.zw;
    if motion.z == 1.0 || motion.z == 2.0 {
        var at = vec2<f32>(dot(p,r.xy),dot(p,r.zw)) / scale.xy * scale.zw;
        if motion.z == 1.0 { at.y += motion.x; at += vec2<f32>(0.5,0.5*scale.w); }
        else { at += vec2<f32>(0.5); }
        return at;
    }
    return vec2<f32>(0.0);
}
fn animation_blend(dest:vec4<f32>, sample:vec4<f32>, mask:vec4<f32>, n:u32) -> vec4<f32> {
    let motion = data[n*5u+3u];
    var src = sample * mask; src.a *= motion.y;
    switch i32(motion.w) {
        case 0: { return vec4<f32>(mix(dest.rgb,dest.rgb+src.rgb,vec3<f32>(src.a)),max(dest.a,src.a)); }
        case 1: { return dest*src; }
        case 2: { return vec4<f32>(mix(dest.rgb*src.rgb*2.0,vec3<f32>(1.0)-2.0*(vec3<f32>(1.0)-dest.rgb)*(vec3<f32>(1.0)-src.rgb),step(vec3<f32>(0.5),dest.rgb)),src.a); }
        case 3: { return vec4<f32>(mix(dest.rgb*dest.a,src.rgb,vec3<f32>(src.a)),max(src.a,dest.a)); }
        default: { return vec4<f32>(1.0,0.0,1.0,1.0); }
    }
}
fn animate(dest:vec4<f32>, mask_uv:vec2<f32>, uv:vec2<f32>, n:u32, mask_page:texture_2d<f32>, page:texture_2d<f32>) -> vec4<f32> {
    // Vanilla animation masks are linear/clamp, animated pages linear/wrap, mip off.
    var mask = textureSampleLevel(mask_page,mask_sampler,mask_uv,0.0);
    var sample = vec4<f32>(data[n*5u+3u].x);
    if data[n*5u+3u].z != 3.0 {
        sample = textureSampleLevel(page,animation_sampler,uv,0.0);
        if data[n*5u+4u].x > 0.0 && (uv.y < 0.0 || uv.y > 1.0) { mask.a = 0.0; }
    }
    return animation_blend(dest,sample,mask,n);
}
"#);
    if apply {
        let mut code =
            "    c = animate(c,uv,in.animated0,0u,slot1,animated_texture0);\n".to_string();
        if records.len() == 2 {
            code.push_str(
                "    c = animate(c,uv,in.animated1,1u,mask_texture1,animated_texture1);\n",
            );
        }
        source = source.replace("    if (((p >> 14u)", &(code + "    if (((p >> 14u)"));
    }
    source
}
