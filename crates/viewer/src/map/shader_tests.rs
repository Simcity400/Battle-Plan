//! Parses and validates the viewer's standalone WGSL with naga, without a device
//! or game content, so a WGSL error fails `cargo test` instead of the first render.
use eframe::wgpu::naga;

/// Several WARP devices used at once in one process fault inside `d3d10warp.dll` (probed
/// 2026-09-26: 5 of 6 runs with three concurrent devices, 0 of 6 with one device shared by three
/// threads). The viewer creates one device per process, so only tests need this. Every GPU test
/// takes it before requesting its adapter, so no second device exists while another test runs.
pub(super) static GPU_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        ("texture_unpack", include_str!("texture_unpack.wgsl")),
        ("sprites", super::sprites::SHADER),
        ("exact sprites", super::sprites::EXACT_SHADER),
    ] {
        validate(name, source);
    }
}
