use super::*;

#[test]
fn a_written_font_viewport_is_the_compiled_projection() {
    let program = content_reader_gui::pdxshader::program::read_with::<Vec<u8>>(b"t.lua", &|_| None).unwrap();
    let quad = |font_viewport| Quad {
        path: String::new(),
        effect: Effect {
            asset: shader_pipeline::library::Effect::from_compiled(
                Arc::new(program.clone()),
                vec![],
                vec![],
            ),
            technique: "Text".into(),
        },
        declaration: Declaration::Text,
        vertices: Vec::new(),
        stride: 28,
        source_draws: 1,
        constants: Matrix::IDENTITY.to_cols_array().to_vec(),
        known: 64,
        pixel_constants: false,
        textures: Vec::new(),
        extra_constants: Vec::new(),
        font_viewport,
        topology: eframe::wgpu::PrimitiveTopology::TriangleList,
        default_blend: false,
        bake: None,
    };
    let plain = quad(None).projected_constants([800, 600]);
    let written = quad(Some([333, 600])).projected_constants([800, 600]);
    assert_eq!((plain[0], plain[5]), (2. / 800., 2. / 600.));
    assert_eq!((written[0], written[5]), (2. / 333., 2. / 600.));
    assert_eq!(written[12..14], [-1., 1.]);
    assert_eq!(gui_viewport([1366, 768], 1.25), [1092, 614]);
}
