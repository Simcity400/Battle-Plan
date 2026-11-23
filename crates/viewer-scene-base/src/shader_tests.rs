use viewer_native_gui::sprites::SHADER;
use crate::shader_validation::validate;

#[test]
fn standalone_map_shaders_validate() {
    for (name, source) in [
        ("post_copy", include_str!("post_copy.wgsl")),
        (
            "texture_unpack",
            include_str!("../../viewer-gpu/src/texture_unpack.wgsl"),
        ),
        ("sprites", SHADER),
    ] {
        validate(name, source);
    }
}
