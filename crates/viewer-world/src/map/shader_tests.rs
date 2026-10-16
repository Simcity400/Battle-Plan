//! Parses and validates the viewer's standalone WGSL with naga, without a device
//! or game content, so a WGSL error fails `cargo test` instead of the first render.
use eframe::wgpu::naga;
use viewer_foundation::map::sprites::SHADER;

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
        ("post_copy", include_str!("post_copy.wgsl")),
        (
            "texture_unpack",
            include_str!("../../../viewer-foundation/src/map/texture_unpack.wgsl"),
        ),
        ("sprites", SHADER),
    ] {
        validate(name, source);
    }
}
