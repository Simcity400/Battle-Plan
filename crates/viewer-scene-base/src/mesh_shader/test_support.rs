use super::*;

pub(super) fn triangle() -> content_reader_mesh::mesh::SubMesh {
    content_reader_mesh::mesh::SubMesh {
        positions: vec![[1., 2., 3.], [4., 5., 6.], [7., 8., 9.]],
        normals: vec![[10., 11., 12.]; 3],
        tangents: vec![[20., 21., 22., 23.]; 3],
        uv0: vec![[30., 31.]; 3],
        indices: vec![0, 1, 2],
        ..Default::default()
    }
}

pub fn fallen_back_pass() -> (
    Pass,
    shader_pipeline::library::Selection,
    Arc<shader_pipeline::library::Effect>,
    Geometry,
) {
    use shader_pipeline::{Compiled, Shader, library::Effect};
    content_reader_script::test_game::enter();
    let input = |location, semantic: &str, components| VertexInput {
        kind: ScalarKind::Float,
        location,
        semantic: semantic.into(),
        components,
    };
    let shader = |wgsl: &str, inputs| Shader {
        wgsl: wgsl.into(),
        independent_draws: false,
        buffers: vec![],
        textures: vec![],
        samplers: vec![],
        sampling_pairs: vec![],
        inputs,
        clock: Default::default(),
    };
    let pixel = shader(
        "@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
        vec![],
    );
    let program = |name: &'static [u8], text: &'static [u8]| {
        Arc::new(
            content_reader_gui::pdxshader::program::read_with(name, &|path| {
                Some(if path == name {
                    text.to_vec()
                } else {
                    Vec::new()
                })
            })
            .unwrap(),
        )
    };
    let recorded = Effect::from_compiled(
        program(
            b"recorded.shader",
            b"BlendState BlendState { BlendEnable = yes SourceBlend = src_alpha DestBlend = inv_src_alpha } Effect Up { }",
        ),
        Vec::new(),
        vec![Ok(Arc::new(Compiled {
            vertex: shader(
                "@vertex fn main(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(p.xy + uv, p.z, 1.0); }",
                vec![input(0, "POSITION", 3), input(1, "TEXCOORD0", 2)],
            ),
            pixel: pixel.clone(),
        }))],
    );
    let replacement = Effect::replacing(
        program(b"mod.shader", b"Effect Up { }"),
        vec![Ok(Arc::new(Compiled {
            vertex: shader(
                "@vertex fn main(@location(0) p: vec3<f32>, @location(1) c: vec4<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(p, 1.0) * c; }",
                vec![input(0, "POSITION", 3), input(1, "COLOR0", 4)],
            ),
            pixel,
        }))],
        recorded.clone(),
    );
    let selection = replacement.select("Up").unwrap().unwrap();
    let geometry = Geometry::from_mesh(&triangle(), 1.0).unwrap();
    let instance = shader_pipeline::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, _queue) =
        pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let pass = Pass::new(
        &mut gpu::Cache::default(),
        &device,
        &selection,
        &geometry,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Depth24PlusStencil8,
        1,
        0,
    )
    .unwrap();
    (pass, selection, recorded, geometry)
}
