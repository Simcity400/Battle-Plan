use eframe::wgpu;
use content_reader_gui::pdxshader::{Address, program::{Filter, Program, Sampler, Technique}};
use shader_pipeline::{
    Buffer, Compiled, ScalarKind, Shader, TextureKind, VertexInput, library::Effect,
};
use std::sync::Arc;
use viewer_native_gui::{native_shader::draw, sprites::{Batch, Exact, Quad}};

pub(super) struct Sprites {
    sample: Arc<Effect>,
    solid: Arc<Effect>,
}

fn effect(sample: bool) -> Arc<Effect> {
    let mut source = Program {
        effect: format!("synthetic-scene-render-{sample}"),
        ..Default::default()
    };
    source.techniques = vec![Technique {
        name: "Up".into(),
        vertex: "V".into(),
        pixel: "P".into(),
        defines: vec![],
        blend: String::new(),
        depth_stencil: String::new(),
        rasterizer: String::new(),
    }];
    if sample {
        source.pixel.samplers.push(Sampler {
            name: "T".into(),
            index: 0,
            cube: false,
            mag: Filter::Point,
            min: Filter::Point,
            mip: Filter::Point,
            u: Address::Clamp,
            v: Address::Clamp,
            w: Address::Wrap,
            lod_bias: 0.,
            min_lod: 0,
            max_lod: 1024,
        });
    }
    let header = "struct Constants { m: mat4x4<f32>, color: vec4<f32>, offsets: vec4<f32>, clocks: vec4<f32> }";
    let shader = |wgsl: String| Shader {
        wgsl,
        independent_draws: false,
        buffers: vec![],
        textures: vec![],
        samplers: vec![],
        sampling_pairs: vec![],
        inputs: vec![],
        clock: Default::default(),
    };
    let mut vertex = shader(format!(
        "{header}\n{}",
        r#"
@group(0) @binding(0) var<uniform> c: Constants;
struct Input { @location(0) position: vec3<f32>, @location(1) uv: vec2<f32> }
struct Output { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn main(v: Input) -> Output { var o: Output; o.position = c.m * vec4<f32>(v.position, 1.0); o.uv = v.uv; return o; }
"#
    ));
    vertex.buffers = vec![Buffer {
        binding: 0,
        size: 112,
        reads: std::iter::once(0..64).collect(),
    }];
    vertex.inputs = [("POSITION", 3), ("TEXCOORD0", 2)]
        .into_iter()
        .enumerate()
        .map(|(i, (semantic, components))| VertexInput {
            kind: ScalarKind::Float,
            location: i as u32,
            semantic: semantic.into(),
            components,
        })
        .collect();
    let body = if sample {
        r#"
@group(1) @binding(1) var image: texture_2d<f32>;
@group(2) @binding(1) var image_sampler: sampler;
@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return textureSample(image, image_sampler, uv) * c.color; }
"#
    } else {
        r#"
@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return c.color; }
"#
    };
    let mut pixel = shader(format!(
        "{header}\n@group(0) @binding(1) var<uniform> c: Constants;\n{body}"
    ));
    pixel.buffers = vec![Buffer {
        binding: 1,
        size: 112,
        reads: std::iter::once(64..80).collect(),
    }];
    if sample {
        pixel.textures = vec![(1, TextureKind::D2)];
        pixel.samplers = vec![1];
        pixel.sampling_pairs = vec![(1, 1)];
    }
    Effect::from_compiled(
        Arc::new(source),
        vec![],
        vec![Ok(Arc::new(Compiled { vertex, pixel }))],
    )
}

fn native(effect: Arc<Effect>, extent: f32, offset: f32) -> draw::Quad {
    let mut constants = vec![0.; 28];
    for i in [0, 5, 10, 15] {
        constants[i] = 1.;
    }
    constants[12] = offset;
    constants[13] = -offset;
    constants[16..20].fill(1.);
    let vertices = [
        0., -extent, 0., 0., 1., 0., 0., 0., 0., 0., extent, -extent, 0., 1., 1., extent, 0., 0.,
        1., 0.,
    ];
    draw::Quad {
        path: "synthetic-scene-render/baked-sprite".into(),
        effect: draw::Effect {
            asset: effect,
            technique: "Up".into(),
        },
        declaration: shader_pipeline::vertices::Declaration::Sprite,
        vertices: bytemuck::cast_slice(&vertices).to_vec(),
        stride: 20,
        source_draws: 1,
        constants,
        known: 112,
        pixel_constants: true,
        textures: vec![],
        extra_constants: vec![],
        font_viewport: None,
        topology: wgpu::PrimitiveTopology::TriangleStrip,
        default_blend: false,
        bake: None,
    }
}

impl Sprites {
    pub(super) fn new() -> Self {
        Self {
            sample: effect(true),
            solid: effect(false),
        }
    }

    pub(super) fn batch(&self, frame: usize) -> Batch {
        let mut background = native(self.solid.clone(), 16., 0.);
        background.constants[16..20].copy_from_slice(if (frame / 2).is_multiple_of(2) {
            &[1., 0., 0., 1.]
        } else {
            &[0., 1., 0., 1.]
        });
        let mut native = native(self.sample.clone(), 12., 2.);
        native.textures.push((0, None));
        native.bake = Some(Arc::new(draw::Bake {
            size: [16, 16],
            draws: vec![background],
        }));
        let quad = Quad {
            texture: viewer_native_gui::sprites::NO_TEXTURE,
            rect: [2., 2., 12., 12.],
            uv: [0., 0., 1., 1.],
            color: [1.; 4],
            rotation: None,
            layer: 0,
            shade: 0.,
            anchor: None,
            present: (frame % 8 == 6).then_some(viewer_native_gui::free_icons::Present {
                pivot: [8., 8.],
                scale: 0.5,
                alpha: 0.5,
                rank: 0,
            }),
            exact: Some(Exact {
                native: Some(Arc::new(native)),
                corners: [[2., 2.], [14., 2.], [14., 14.], [2., 14.]],
                uv: [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
                scissor: None,
                technique: 0,
                blend: viewer_native_gui::native_effects::Blend::Off,
                samplers: 0,
                lod_bias: [0.; 3],
                second: None,
                masking: None,
                offsets: [0.; 4],
                inputs: [0.; 4],
                second_color: [0.; 4],
            }),
        };
        let mut batch = Batch::default();
        batch.quads.push(quad);
        batch
    }
}
