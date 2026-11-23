use super::*;
use crate::{native_shader::draw, sprites::{self, Batch, Exact, Quad, SpritePass}};
use eframe::wgpu;
use content_reader_gui::pdxshader::{Address, program::{Filter, Program, Sampler, Technique as AuthoredTechnique}};
use shader_pipeline::{Buffer, Compiled, ScalarKind, Shader, TextureKind, VertexInput};

fn program() -> Program {
    let mut source = Program {
        effect: "synthetic-packets".into(),
        ..Default::default()
    };
    source.techniques = ["Up", "Failed"]
        .map(|name| AuthoredTechnique {
            name: name.into(),
            vertex: "V".into(),
            pixel: "P".into(),
            defines: vec![],
            blend: String::new(),
            depth_stencil: String::new(),
            rasterizer: String::new(),
        })
        .into();
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
    source
}

fn shader(wgsl: &str) -> Shader {
    Shader {
        wgsl: wgsl.into(),
        independent_draws: false,
        buffers: vec![],
        textures: vec![],
        samplers: vec![],
        sampling_pairs: vec![],
        inputs: vec![],
        clock: Default::default(),
    }
}

fn effect() -> Arc<Effect> {
    let mut vertex = shader(
        r#"
struct Constants { m: mat4x4<f32>, color: vec4<f32>, offsets: vec4<f32>, clocks: vec4<f32> }
@group(0) @binding(0) var<uniform> c: Constants;
struct Input { @location(0) position: vec3<f32>, @location(1) uv: vec2<f32> }
struct Output { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn main(v: Input) -> Output { var o: Output; o.position = c.m * vec4<f32>(v.position, 1.0); o.uv = v.uv; return o; }
"#,
    );
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
    let mut pixel = shader(
        r#"
struct Constants { m: mat4x4<f32>, color: vec4<f32>, offsets: vec4<f32>, clocks: vec4<f32> }
@group(0) @binding(1) var<uniform> c: Constants;
@group(1) @binding(1) var image: texture_2d<f32>;
@group(2) @binding(1) var image_sampler: sampler;
@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return textureSample(image, image_sampler, uv) * c.color; }
"#,
    );
    pixel.buffers = vec![Buffer {
        binding: 1,
        size: 112,
        reads: std::iter::once(64..80).collect(),
    }];
    pixel.textures = vec![(1, TextureKind::D2)];
    pixel.samplers = vec![1];
    pixel.sampling_pairs = vec![(1, 1)];
    Effect::from_compiled(
        Arc::new(program()),
        vec![],
        vec![
            Ok(Arc::new(Compiled { vertex, pixel })),
            Err("synthetic selection failed".into()),
        ],
    )
}

fn draw(asset: Arc<Effect>, path: &str) -> draw::Quad {
    let mut constants = vec![0.; 28];
    for i in [0, 5, 10, 15] {
        constants[i] = 1.;
    }
    constants[12] = 2.;
    constants[13] = -2.;
    constants[16..20].fill(1.);
    let vertices: Vec<f32> = [
        [0., -12., 0., 0., 1.],
        [0., 0., 0., 0., 0.],
        [12., -12., 0., 1., 1.],
        [12., 0., 0., 1., 0.],
    ]
    .into_iter()
    .flatten()
    .collect();
    draw::Quad {
        source_draws: 1,
        path: path.into(),
        effect: draw::Effect {
            asset,
            technique: "Up".into(),
        },
        declaration: shader_pipeline::vertices::Declaration::Sprite,
        vertices: bytemuck::cast_slice(&vertices).to_vec(),
        stride: 20,
        constants,
        known: 112,
        pixel_constants: true,
        textures: vec![(0, Some(0))],
        extra_constants: vec![],
        font_viewport: None,
        topology: wgpu::PrimitiveTopology::TriangleStrip,
        default_blend: false,
        bake: None,
    }
}

#[test]
fn sprite_packet_metadata_keys_and_live_retirement_preserve_immutable_handles() {
    let mut cache = Cache::default();
    let mut draw = draw(effect(), "root/a");
    let shared = |text: Text| match text {
        Text::Shared(s) => s,
        Text::Fresh(_) => panic!("cached metadata should share its label"),
    };
    let first = cache.metadata(&draw);
    let label = shared(first.label);
    assert!(first.selection.unwrap().is_some());
    for _ in 0..8 {
        cache.begin();
        let hit = cache.metadata(&draw);
        assert!(Arc::ptr_eq(&label, &shared(hit.label)));
        cache.finish();
    }
    draw.path = "root/b".into();
    let second = cache.metadata(&draw);
    assert!(!Arc::ptr_eq(&label, &shared(second.label)));
    draw.effect.technique = "up".into();
    assert_eq!(cache.metadata(&draw).selection.unwrap().unwrap().index, 0);
    draw.effect.technique = "Missing".into();
    assert!(cache.metadata(&draw).selection.unwrap().is_none());
    let missing = shared(cache.metadata(&draw).label);
    assert!(Arc::ptr_eq(&missing, &shared(cache.metadata(&draw).label)));
    draw.effect.asset = effect();
    assert!(!Arc::ptr_eq(&missing, &shared(cache.metadata(&draw).label)));
    cache.begin();
    cache.metadata(&draw);
    cache.finish();
    assert_eq!(cache.effects.len(), 1);
    assert_eq!(cache.effects.values().next().unwrap().techniques.len(), 1);
    assert_eq!(&*label, "root/a / synthetic-packets:Up");
    cache.begin();
    cache.finish();
    assert!(cache.effects.is_empty());
}

struct Fail;
impl shader_pipeline::cache::Compile for Fail {
    fn compile(
        &self,
        _: &Program,
        _: &AuthoredTechnique,
        _: &[String],
    ) -> Result<Compiled, String> {
        Err("synthetic compile failed".into())
    }
}

#[test]
fn sprite_packet_failed_selects_stay_uncached_and_replay_every_refusal() {
    let diagnostics = Arc::new(shader_pipeline::cache::Cache::new(None, Arc::new(Fail)));
    let mut source = program();
    for (stage, name) in [(&mut source.vertex, "V"), (&mut source.pixel, "P")] {
        stage
            .entries
            .push(content_reader_gui::pdxshader::program::Entry {
                name: name.into(),
                source: Default::default(),
            });
    }
    let effect = Effect::new(Arc::new(source), vec![], diagnostics.clone());
    let mut cache = Cache::default();
    for frame in 0..8 {
        cache.begin();
        for _ in 0..3 {
            let metadata = cache.metadata(&draw(effect.clone(), "failed"));
            assert_eq!(&*metadata.label, "failed / synthetic-packets:Up");
            assert_eq!(metadata.selection.unwrap_err(), "synthetic compile failed");
        }
        cache.finish();
        assert!(cache.effects.is_empty());
        assert_eq!(
            diagnostics.diagnostics().refused,
            [("synthetic compile failed".into(), (frame + 1) * 3)].into()
        );
    }
}

#[test]
fn sprite_packet_view_capacity_never_reuses_resolution_nulls_missing_or_revisions() {
    let mut draw = draw(effect(), "views");
    draw.textures = vec![(0, Some(3)), (2, None), (5, Some(9))];
    let mut reused = Vec::new();
    for generation in 0..8 {
        let resolve = |id| (id != 9 || generation % 2 == 1).then_some(id as u64 + generation * 100);
        let mut calls = Vec::new();
        views(&mut reused, &draw, |id| {
            calls.push(id);
            resolve(id)
        });
        assert_eq!(calls, [3, 9]);
        assert_eq!(
            reused,
            vec![
                (0, Ok(Some(3 + generation * 100))),
                (2, Ok(None)),
                (5, resolve(9).map(Some).ok_or_else(||
                    "effect \"synthetic-packets\", draw \"views\": texture 9 is not resident".into()
                )),
            ]
        );
        assert!(reused.capacity() >= 3);
    }
    draw.textures.clear();
    views(&mut reused, &draw, |_| None::<u64>);
    assert!(reused.is_empty());
    assert!(reused.capacity() >= 3);
}

fn quad(native: draw::Quad) -> Quad {
    Quad {
        texture: 0,
        rect: [2., 2., 12., 12.],
        uv: [0., 0., 1., 1.],
        color: [1.; 4],
        rotation: None,
        layer: 0,
        shade: 0.,
        anchor: None,
        present: None,
        exact: Some(Exact {
            native: Some(Arc::new(native)),
            corners: [[2., 2.], [14., 2.], [14., 14.], [2., 14.]],
            uv: [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
            scissor: None,
            technique: 0,
            blend: crate::native_effects::Blend::Off,
            samplers: 0,
            lod_bias: [0.; 3],
            second: None,
            masking: None,
            offsets: [0.; 4],
            inputs: [0.; 4],
            second_color: [0.; 4],
        }),
    }
}

fn expected_pixels(frame: usize) -> Vec<u8> {
    let colour = match frame {
        0 | 1 | 4 | 5 | 9 => [255, 0, 0, 255],
        2 | 3 => [0, 255, 0, 255],
        6 | 7 | 10 | 11 => [128, 0, 0, 255],
        8 => [64, 0, 0, 255],
        _ => unreachable!(),
    };
    let mut pixels = vec![0; 16 * 16 * 4];
    let (first, last, middle, split, repair) = if frame == 10 {
        (3, 16, 10, 15, 15)
    } else {
        (2, 14, 8, 12, 12)
    };
    for y in 0..16 {
        for x in 0..16 {
            let mut pixel = [0; 4];
            if frame == 6 {
                if (5..11).contains(&x) && (5..11).contains(&y) {
                    pixel = [64, 0, 0, 0];
                }
            } else if (first..last).contains(&x) && (first..last).contains(&y) {
                pixel = colour;
            }
            if (first..last).contains(&x) && (first..last).contains(&y) {
                if x >= middle && y < middle {
                    pixel = colour;
                }
                if (middle..split).contains(&x) && y >= middle {
                    pixel = [0; 4];
                }
                if x >= repair && y >= middle {
                    pixel = [255, 0, 0, 255];
                }
            }
            let at = (y * 16 + x) * 4;
            pixels[at..at + 4].copy_from_slice(&pixel);
        }
    }
    pixels
}

#[test]
fn sprite_packet_multiframe_pixels_refusals_bakes_and_arenas_match_fixed_expectations() {
    let (_serial, device, queue) = viewer_gpu::test_support::gpu::device();
    let pages: Vec<_> = [[255, 0, 0, 255], [0, 255, 0, 255]]
        .into_iter()
        .map(|c| {
            content_reader_image::dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
                width: 1,
                height: 1,
                pixels: c.to_vec(),
            })
        })
        .collect();
    let images = sprites::IconTextures::new(&device, &queue, &pages).unwrap();
    let mut pass = SpritePass::new(
        &device,
        &queue,
        &images,
        &crate::lazy_icons::LazyTextures::new(),
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let base = draw(effect(), "root/native");
    let mut background = base.clone();
    background.constants[12..14].fill(0.);
    background.vertices = bytemuck::cast_slice(&[
        0_f32, -16., 0., 0., 1., 0., 0., 0., 0., 0., 16., -16., 0., 1., 1., 16., 0., 0., 1., 0.,
    ])
    .to_vec();
    let expected_refusals: std::collections::BTreeMap<String, u64> = [
        ("synthetic selection failed".into(), 1),
        ("native draw \"root/after-failure / synthetic-packets:Absent\": null technique needs the preceding native program, which has not been supplied".into(), 1),
        ("effect \"synthetic-packets\", draw \"root/unavailable\": texture 999 is not resident".into(), 1),
    ].into();
    let expected_arena_refusals: std::collections::BTreeSet<String> = [
        "GUI shader \"root/failed\": effect \"synthetic-packets\" technique \"Failed\"",
        "GUI shader \"root/after-failure\": effect \"synthetic-packets\" technique \"Absent\"",
        "GUI shader \"root/unavailable\": effect \"synthetic-packets\" technique \"Up\"",
        "native draw \"root/after-failure / synthetic-packets:Absent\": no program",
    ]
    .map(str::to_owned)
    .into();
    let mut previous_bake = None;
    let mut private_frame = 0;
    for frame in 0..12 {
        let state = frame / 2;
        private_frame += u64::from(!matches!(frame, 1 | 3 | 5));
        let mut a = base.clone();
        a.constants[16] = if state % 2 == 0 { 1. } else { 128. / 255. };
        a.constants[24] = state as f32;
        a.textures[0].1 = Some(state % 2);
        if frame >= 4 {
            a.bake = Some(Arc::new(draw::Bake {
                size: [16, 16],
                draws: vec![background.clone()],
            }));
        }
        if frame == 8 {
            Arc::make_mut(a.bake.as_mut().unwrap()).draws[0].constants[16] = 0.25;
        }
        let mut missing = a.clone();
        missing.path = "root/missing".into();
        missing.effect.technique = "Absent".into();
        missing.textures.clear();
        missing.bake = None;
        let mut failed = missing.clone();
        failed.path = "root/failed".into();
        failed.effect.technique = "Failed".into();
        let mut after = missing.clone();
        after.path = "root/after-failure".into();
        let mut unavailable = base.clone();
        unavailable.path = "root/unavailable".into();
        unavailable.textures[0].1 = Some(999);
        let mut null = base.clone();
        null.path = "root/null".into();
        null.textures[0].1 = None;
        let mut batch = Batch {
            quads: vec![
                quad(a),
                quad(missing),
                quad(failed),
                quad(after),
                quad(unavailable),
                quad(null),
                quad(base.clone()),
            ],
            ..Batch::default()
        };
        batch.quads[1].exact.as_mut().unwrap().scissor = Some([8, 0, 16, 8]);
        batch.quads[5].exact.as_mut().unwrap().scissor = Some([8, 8, 12, 16]);
        batch.quads[6].exact.as_mut().unwrap().scissor = Some([12, 8, 16, 16]);
        if frame == 6 {
            batch.quads[0].present = Some(crate::free_icons::Present {
                pivot: [8., 8.],
                scale: 0.5,
                alpha: 0.5,
                rank: 0,
            });
        }
        arena.reset();
        arena.vertices(&[0xff; 4096]).unwrap();
        pass.upload(
            &device,
            &queue,
            &arena,
            &batch,
            [16, 16],
            if frame == 10 { 1.25 } else { 1. },
        );
        arena.flush(&queue);
        let mut encoder = device.create_command_encoder(&Default::default());
        pass.draw_pages(&mut encoder);
        queue.submit(pass.bake_buffers().into_iter().chain([encoder.finish()]));
        let pixels = viewer_gpu::test_support::gpu::render_with(&device, &queue, |render| {
            pass.draw_runs(render, [16, 16])
        });
        assert_eq!(pixels, expected_pixels(frame), "frame {frame}");
        assert_eq!(pass.native_refusals, expected_refusals, "frame {frame}");
        assert_eq!(arena.refusals(), expected_arena_refusals, "frame {frame}");
        assert_eq!(pass.native.cache.created, 1, "frame {frame}");
        assert_eq!(
            pass.bake_stream.cache.created,
            usize::from(frame >= 4),
            "frame {frame}"
        );
        assert_eq!(
            pass.fade_draws.len(),
            usize::from(frame == 6),
            "frame {frame}"
        );
        assert_eq!(pass.bakes.len(), usize::from(frame >= 4), "frame {frame}");
        assert_eq!(
            pass.uploads.as_ref().unwrap().frame(),
            private_frame,
            "frame {frame}"
        );
        assert!(pass.retained_valid, "frame {frame}");
        assert_eq!(pass.fade_pages, usize::from(frame == 6));
        assert_eq!(pass.native_draws.len(), if frame == 6 { 3 } else { 4 });
        if frame >= 4 {
            let current = &pass.bakes[0].view;
            if let Some(previous) = &previous_bake {
                if matches!(frame, 8 | 9) {
                    assert_ne!(previous, current, "changed bake recipe at frame {frame}");
                } else {
                    assert_eq!(previous, current, "unchanged bake reused at frame {frame}");
                }
            }
            previous_bake = Some(current.clone());
        }
        assert!(pass.packet_views.is_empty() && pass.bake_views.is_empty());
    }
    assert_eq!(pass.packets.effects.len(), 1);
    assert!(
        !pass
            .packets
            .effects
            .values()
            .next()
            .unwrap()
            .techniques
            .contains_key("Failed")
    );
    assert!(pass.packet_views.capacity() >= 1);
    assert!(pass.bake_views.capacity() >= 1);
}
