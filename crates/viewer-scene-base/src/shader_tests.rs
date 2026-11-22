use viewer_foundation::map::sprites::SHADER;
use crate::shader_validation::validate;

#[test]
fn standalone_map_shaders_validate() {
    for (name, source) in [
        ("post_copy", include_str!("post_copy.wgsl")),
        (
            "texture_unpack",
            include_str!("../../viewer-foundation/src/map/texture_unpack.wgsl"),
        ),
        ("sprites", SHADER),
    ] {
        validate(name, source);
    }
}
