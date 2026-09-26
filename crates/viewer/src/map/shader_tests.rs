//! Parses and validates every map shader with naga, without a device or game
//! content, so a WGSL error fails `cargo test` instead of the first render.
use eframe::wgpu::naga;

fn validate(name: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|error| panic!("{name}: {}", error.emit_to_string(source)));
}

#[test]
fn standalone_map_shaders_validate() {
    for (name, source) in [
        ("political_fill", include_str!("political_fill.wgsl")),
        ("post", include_str!("post.wgsl")),
        ("post_luts", include_str!("post_luts.wgsl")),
        ("shadow", include_str!("shadow.wgsl")),
        ("sprites", super::sprites::SHADER),
    ] {
        validate(name, source);
    }
}

#[test]
fn world_shaders_validate_with_shared_frame_inputs() {
    for (name, source) in [
        ("border", include_str!("border.wgsl")),
        ("maparrow", include_str!("maparrow.wgsl")),
        ("mapframe", include_str!("mapframe.wgsl")),
        ("mapname", include_str!("mapname.wgsl")),
        ("pdxmesh", include_str!("pdxmesh.wgsl")),
        ("railroad", include_str!("railroad.wgsl")),
        ("river", include_str!("river.wgsl")),
        ("terrain", include_str!("terrain.wgsl")),
        ("tree", include_str!("tree.wgsl")),
        ("water", include_str!("water.wgsl")),
        ("weather_effects", include_str!("weather_effects.wgsl")),
    ] {
        // This is the same composition used by scene::module.
        validate(name, &format!("{}\n{source}", include_str!("common.wgsl")));
    }
}
