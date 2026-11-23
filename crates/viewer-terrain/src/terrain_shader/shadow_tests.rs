#[test]
fn a_program_reading_the_screen_buffer_by_texel_keeps_it_full_size() {
    use content_reader_gui::pdxshader::program::Stage;
    use shader_pipeline::binding;
    use viewer_scene_base::shadow::texel_reads;
    let reads = |slot: u32, technique: &str, body: &str| {
        let (own, other) = (binding(slot, Stage::Pixel), binding(slot + 1, Stage::Pixel));
        let wgsl = format!(
            "@group(1) @binding({own}) var screen: texture_2d<f32>;
             @group(1) @binding({other}) var other: texture_2d<f32>;
             @group(2) @binding({own}) var linear: sampler;
             @fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{
                 {body}
             }}"
        );
        let stage = shader_pipeline::Shader {
            wgsl,
            independent_draws: false,
            buffers: Vec::new(),
            textures: vec![
                (own, shader_pipeline::TextureKind::D2),
                (other, shader_pipeline::TextureKind::D2),
            ],
            samplers: vec![own],
            sampling_pairs: vec![(own, own), (other, own)],
            inputs: Vec::new(),
            clock: Default::default(),
        };
        texel_reads(
            &shader_pipeline::Compiled {
                vertex: shader_pipeline::Shader {
                    wgsl: String::new(),
                    textures: Vec::new(),
                    samplers: Vec::new(),
                    sampling_pairs: Vec::new(),
                    ..stage.clone()
                },
                pixel: stage,
            },
            slot,
            technique,
        )
    };
    for (slot, technique) in [
        (crate::terrain_shader::SHADOW_SLOT, "terrain"),
        (viewer_scene_base::shadow_blur::SOURCE_SLOT, "ShadowBlurHorizontal"),
    ] {
        assert_eq!(
            reads(slot, technique, "return textureSample(screen, linear, uv);"),
            None
        );
        assert_eq!(
            reads(
                slot,
                technique,
                "return textureSample(other, linear, uv, vec2<i32>(1, 0));"
            ),
            None,
            "another texture's offsets"
        );
        assert_eq!(
            reads(
                slot,
                technique,
                "return textureSample(screen, linear, uv, vec2<i32>(1, 0));"
            )
            .as_deref(),
            Some(
                format!(
                    "half-size shadow buffer not used: the {technique} technique reads the \
                     shadow buffer by texel: it samples it at texel offsets"
                )
                .as_str()
            )
        );
        assert_eq!(
            reads(
                slot,
                technique,
                "let texel = 1.0 / vec2<f32>(textureDimensions(screen));
                 return textureSample(screen, linear, uv + texel);"
            )
            .as_deref(),
            Some(
                format!(
                    "half-size shadow buffer not used: the {technique} technique reads the \
                     shadow buffer by texel: it reads its own size"
                )
                .as_str()
            )
        );
    }
}
