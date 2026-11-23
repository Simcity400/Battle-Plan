use super::*;
use crate::{native_gui as gui, sprites};
use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_image::dds;
use std::sync::Arc;

pub(super) fn effects() -> (synthetic_game::Built, library::Library) {
    let install = synthetic_game::Installation::new("native-gui-shaders")
        .file("gfx/FX/defines_hlsl_dx11.fxh", "")
        .file("gfx/FX/buttonstate.shader", BUTTON)
        .file("gfx/FX/circularprogressbar.shader", CIRCULAR)
        .file("gfx/FX/text.shader", TEXT)
        .file("gfx/FX/progress.shader", PROGRESS)
        .file("gfx/FX/maskedflag.shader", FLAG)
        .build();
    let layers = Arc::new(ResolvedLayers::from_folders(&[(&install.game_dir, &[])]).unwrap());
    (install, library::Library::new(layers))
}

const BUTTON: &str = r#"
VertexStruct In {
    float3 at : POSITION;
    float2 uv : TEXCOORD0;
};
VertexStruct Out {
    float4 at : SV_POSITION;
    float2 uv : TEXCOORD0;
};
ConstantBuffer(0, 0) {
    float4x4 world;
    float4 tint;
    float4 offset;
};
ConstantBuffer(1, 1) {
    float4 records[10];
};
VertexShader = { MainCode V [[ Out main(In v) {
    Out o; o.at = mul(world, float4(v.at, 1)); o.uv = v.uv + offset.xy; return o;
} ]] }
PixelShader = {
    Samplers = {
        Base = { Index = 0 }
        Mask = { Index = 1 }
        Frame = { Index = 2 }
        NextMask = { Index = 3 }
        NextFrame = { Index = 4 }
        Clip = { Index = 5 }
    }
    MainCode P [[ float4 main(Out v) : SV_TARGET {
        float4 c = Base_Texture.Sample(Base_Sampler, v.uv);
#ifdef ANIMATED
        c = lerp(c, Frame_Texture.Sample(Frame_Sampler, v.uv),
                 Mask_Texture.Sample(Mask_Sampler, v.uv).r * records[3].z * 0.25);
#ifdef NUM_ANIMATIONS_2
        c = lerp(c, NextFrame_Texture.Sample(NextFrame_Sampler, v.uv),
                 NextMask_Texture.Sample(NextMask_Sampler, v.uv).r * records[8].z * 0.25);
#endif
#endif
#ifdef MASKING
        c.a *= Clip_Texture.Sample(Clip_Sampler, v.uv).r;
#endif
#ifdef DOWN_STATE
        c.r *= 0.5;
#endif
#ifdef OVER_STATE
        c.g *= 0.5;
#endif
#ifdef DISABLED_STATE
        c.b *= 0.5;
#endif
        return c * tint;
    } ]]
}
Effect Up { VertexShader = V PixelShader = P }
Effect Down { VertexShader = V PixelShader = P Defines = { DOWN_STATE } }
Effect Over { VertexShader = V PixelShader = P Defines = { OVER_STATE } }
Effect Disable { VertexShader = V PixelShader = P Defines = { DISABLED_STATE } }
"#;

const CIRCULAR: &str = r#"
VertexStruct In {
    float2 at : POSITION;
    float2 uv : TEXCOORD0;
    float2 slot : TEXCOORD1;
};
VertexStruct Out {
    float4 at : SV_POSITION;
    float2 uv : TEXCOORD0;
    float slot : TEXCOORD1;
};
VertexShader = {
    ConstantBuffer(0, 0) {
        float4x4 world;
    };
    MainCode V [[ Out main(In v) {
        Out o; o.at = mul(world, float4(v.at, 0, 1)); o.uv = v.uv * 0.5 + 0.5;
        o.slot = v.slot.x; return o;
    } ]]
}
PixelShader = {
    Samplers = { A = { Index = 0 } B = { Index = 1 } }
    MainCode P [[ float4 main(Out v) : SV_TARGET {
        return lerp(A_Texture.Sample(A_Sampler, v.uv), B_Texture.Sample(B_Sampler, v.uv), v.slot);
    } ]]
}
Effect CircularProgressBar { VertexShader = V PixelShader = P }
"#;

const TEXT: &str = r#"
VertexStruct In {
    float3 at : POSITION;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
};
VertexStruct Out {
    float4 at : SV_POSITION;
    float2 uv : TEXCOORD0;
    float4 color : TEXCOORD1;
};
VertexShader = {
    ConstantBuffer(0, 0) {
        float4x4 world;
        float4 tint;
    };
    MainCode V [[ Out main(In v) {
        Out o; o.at = mul(world, float4(v.at, 1)); o.uv = v.uv; o.color = v.color * tint; return o;
    } ]]
}
PixelShader = {
    Samplers = { Page = { Index = 0 } }
    MainCode P [[ float4 main(Out v) : SV_TARGET { return Page_Texture.Sample(Page_Sampler, v.uv) * v.color; } ]]
}
Effect Text { VertexShader = V PixelShader = P }
"#;

const PROGRESS: &str = r#"
VertexStruct In {
    float3 at : POSITION;
    float2 uv : TEXCOORD0;
};
VertexStruct Out {
    float4 at : SV_POSITION;
    float2 uv : TEXCOORD0;
};
ConstantBuffer(0, 0) {
    float4x4 world;
    float4 a;
    float4 b;
    float amount;
};
VertexShader = { MainCode V [[ Out main(In v) {
    Out o; o.at = mul(world, float4(v.at, 1)); o.uv = v.uv; return o;
} ]] }
PixelShader = {
    Samplers = { First = { Index = 0 } Second = { Index = 1 } }
    MainCode P [[ float4 main(Out v) : SV_TARGET {
        return v.uv.x < amount ? a + First_Texture.Sample(First_Sampler, v.uv)
                               : b + Second_Texture.Sample(Second_Sampler, v.uv);
    } ]]
}
Effect Up { VertexShader = V PixelShader = P }
"#;

const FLAG: &str = r#"
VertexStruct In {
    float3 at : POSITION;
    float2 uv : TEXCOORD0;
};
VertexStruct Out {
    float4 at : SV_POSITION;
    float2 uv : TEXCOORD0;
};
VertexShader = {
    ConstantBuffer(0, 0) {
        float4x4 world;
        float4 bounds;
    };
    MainCode V [[ Out main(In v) {
        Out o; o.at = mul(world, float4(v.at, 1)); o.uv = bounds.xy + v.uv * bounds.zw; return o;
    } ]]
}
PixelShader = {
    Samplers = { Flag = { Index = 0 } Mask = { Index = 1 } }
    MainCode P [[ float4 main(Out v) : SV_TARGET {
        return Flag_Texture.Sample(Flag_Sampler, v.uv) * Mask_Texture.Sample(Mask_Sampler, v.uv);
    } ]]
}
Effect Up { VertexShader = V PixelShader = P }
"#;


pub(super) struct Sheets;
impl gui::Sheets for Sheets {
    fn sheet(&self, _: &[u8]) -> Option<&sprites::SpriteSheet> {
        Some(&PRIMARY)
    }
    fn progress(&self, _: &[u8]) -> Option<[&sprites::SpriteSheet; 2]> {
        Some([&PRIMARY, &SECONDARY])
    }
    fn masking(&self, _: &[u8]) -> Option<&sprites::SpriteSheet> {
        Some(&MASK)
    }
    fn mask(&self, _: &[u8]) -> Option<&sprites::SpriteSheet> {
        Some(&SECONDARY)
    }
}
static PRIMARY: sprites::SpriteSheet = sprites::SpriteSheet {
    name: String::new(),
    texture: 0,
    frames: 1,
    size: [16, 16],
};
static SECONDARY: sprites::SpriteSheet = sprites::SpriteSheet {
    name: String::new(),
    texture: 1,
    frames: 1,
    size: [16, 16],
};
static MASK: sprites::SpriteSheet = sprites::SpriteSheet {
    name: String::new(),
    texture: 5,
    frames: 1,
    size: [16, 16],
};

pub(super) fn geometry() -> gui::Geometry {
    gui::Geometry {
        rect: [0., 0., 16., 16.],
        local: [0., 0., 16., 16.],
        pivot: [0., 16.],
        rotation: 0.,
    }
}

pub(super) fn part(kind: gui::PartKind, asset: Arc<library::Effect>, technique: &str) -> gui::Part {
    gui::Part {
        path: "synthetic GUI submission fixture".into(),
        child: None,
        background_index: None,
        kind,
        clip: None,
        color: [1.; 4],
        native: None,
        shading: gui::Shading {
            compiled: Some(draw::Effect {
                asset,
                technique: technique.into(),
            }),
            fixture: None,
            since: 0.125,
            animation: 0.,
            time: 0.125,
            clocked: true,
        },
    }
}

pub(super) fn views(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pages: &[dds::Texture],
) -> Vec<wgpu::TextureView> {
    pages
        .iter()
        .map(|page| {
            viewer_gpu::textures::upload(
                device,
                queue,
                "reference texels",
                page,
                viewer_gpu::textures::ColorSpace::Linear,
            )
            .unwrap()
            .texture
            .create_view(&Default::default())
        })
        .collect()
}

pub(super) fn matrix(size: [u32; 2], translation: [f32; 3]) -> Vec<f32> {
    let sx = 2. / size[0] as f32;
    let sy = 2. / size[1] as f32;
    vec![
        sx,
        0.,
        0.,
        0.,
        0.,
        sy,
        0.,
        0.,
        0.,
        0.,
        1.,
        0.,
        sx * translation[0] - 1.,
        sy * translation[1] + 1.,
        translation[2],
        1.,
    ]
}

pub(super) fn rectangle(
    rect: [f32; 4],
    uv: [f32; 4],
    order: &[usize],
    z: f32,
    packed: Option<([f32; 4], u32)>,
) -> Vec<u8> {
    let [x, y, width, height] = rect;
    let positions = [
        [x, y + height],
        [x + width, y + height],
        [x + width, y],
        [x, y],
    ];
    let coords = [
        [uv[0], uv[1]],
        [uv[2], uv[1]],
        [uv[2], uv[3]],
        [uv[0], uv[3]],
    ];
    let mut bytes = Vec::new();
    for &corner in order {
        for value in [
            positions[corner][0],
            positions[corner][1],
            z,
            coords[corner][0],
            coords[corner][1],
        ] {
            bytes.extend(value.to_le_bytes());
        }
        if let Some((color, border)) = packed {
            bytes.extend([
                (color[2] * 255.) as u8,
                (color[1] * 255.) as u8,
                (color[0] * 255.) as u8,
                (color[3] * 255.) as u8,
            ]);
            bytes.extend(border.to_le_bytes());
        }
    }
    bytes
}

#[allow(clippy::too_many_arguments)]
pub(super) fn inputs(
    effect: &Arc<library::Effect>,
    technique: &str,
    vertices: Vec<u8>,
    declaration: vertices::Declaration,
    stride: usize,
    constants: Vec<f32>,
    known: u32,
    pixel_constants: bool,
    textures: &[(u32, usize)],
    views: &[wgpu::TextureView],
    topology: wgpu::PrimitiveTopology,
    format: wgpu::TextureFormat,
) -> sprites::oracle::Inputs {
    let state =
        gpu::State::authored(&effect.source, effect.source.technique(technique).unwrap()).unwrap();
    let mut bound = BTreeMap::new();
    let mut initialized = BTreeMap::new();
    for stage in [Stage::Vertex, Stage::Pixel] {
        if stage == Stage::Pixel && !pixel_constants {
            continue;
        }
        let slot = binding(0, stage);
        bound.insert(slot, bytemuck::cast_slice(&constants).to_vec());
        initialized.insert(slot, std::iter::once(0..known).collect());
    }
    sprites::oracle::Inputs {
        declaration,
        vertices,
        stride,
        constants: bound,
        known_constants: initialized,
        textures: textures
            .iter()
            .map(|&(slot, image)| (binding(slot, Stage::Pixel), views[image].clone()))
            .collect(),
        topology,
        state,
        format,
    }
}

pub(super) fn has_sampler_slots(effect: &Arc<library::Effect>, technique: &str) -> bool {
    effect.source.technique(technique).is_some()
        && effect
            .source
            .common
            .samplers
            .iter()
            .chain(&effect.source.pixel.samplers)
            .next()
            .is_some()
}

pub(super) fn animation_inputs(inputs: &mut sprites::oracle::Inputs, records: &[[[f32; 4]; 5]]) {
    let mut constants = vec![0.; 64];
    let mut known = vec![0..48, 80..128];
    for (record, values) in records.iter().enumerate() {
        for (row, values) in values.iter().enumerate() {
            constants[record * 20 + row * 4..record * 20 + row * 4 + 4].copy_from_slice(values);
        }
        known.push(record as u32 * 80..record as u32 * 80 + 68);
    }
    for stage in [Stage::Vertex, Stage::Pixel] {
        let slot = binding(1, stage);
        inputs
            .constants
            .insert(slot, bytemuck::cast_slice(&constants).to_vec());
        inputs.known_constants.insert(slot, known.clone());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Sprite,
    Progress,
    Text,
    Flag,
    Circular,
}
impl Family {
    fn for_file(file: &str) -> Self {
        match file {
            "text.shader" => Self::Text,
            "maskedflag.shader" => Self::Flag,
            "circularprogressbar.shader" => Self::Circular,
            name if name.starts_with("progress") => Self::Progress,
            _ => Self::Sprite,
        }
    }
}

#[test]
#[ignore = "requires the shader compiler and GPU"]
fn synthetic_gui_parts_match_the_direct_shader_submissions() {
    content_reader_script::test_game::enter();
    let (_install, mut library) = effects();
    if let Some(report) = std::env::var_os("BATTLEPLAN_SHADER_REPORT").or_else(|| std::env::var_os("HOI4_SHADER_REPORT")) {
        std::fs::create_dir_all(report).unwrap();
    }
    let (_serial, device, queue) = device();
    let arena = gpu::Arena::new(&device);
    let mut pages: Vec<_> = (0..6).map(texture).collect();
    pages.push(dds::Texture::from_rgba8(content_reader_image::Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![0; 4],
    }));
    let views = views(&device, &queue, &pages);
    let mut rows = Vec::new();
    let mut failures = 0;
    let mut compared = 0;
    for name in [
        "buttonstate.shader",
        "progress.shader",
        "text.shader",
        "maskedflag.shader",
        "circularprogressbar.shader",
    ] {
        let family = Family::for_file(name);
        let file = format!("gfx/FX/{name}");
        let base = library.load(file.as_bytes(), &[]).unwrap();
        assert!(
            !base.source.techniques.is_empty(),
            "{file} declares no techniques: {:?}",
            base.source.warnings
        );
        for defines in variants(&base.source) {
            let animated = defines.iter().any(|d| d == "ANIMATED");
            let masked = defines.iter().any(|d| d == "MASKING");
            let count = if defines.iter().any(|d| d == "NUM_ANIMATIONS_2") {
                2
            } else {
                1
            };
            let requested = base.variant(defines.clone());
            let reference = if animated {
                let mut caller: Vec<_> = defines
                    .iter()
                    .filter(|d| {
                        *d != "MASKING" && *d != "ANIMATED" && !d.starts_with("NUM_ANIMATIONS_")
                    })
                    .cloned()
                    .collect();
                caller.extend(["ANIMATED".into(), format!("NUM_ANIMATIONS_{count}")]);
                base.variant(caller)
            } else {
                requested.clone()
            };
            for technique in &base.source.techniques {
                let technique_name: &'static str =
                    Box::leak(technique.name.clone().into_boxed_str());
                let active_animation = animated
                    && family == Family::Sprite
                    && has_sampler_slots(&reference, &technique.name);
                let selected_reference = if active_animation {
                    &reference
                } else {
                    &requested
                };
                let scenarios = if animated {
                    12
                } else if family == Family::Progress {
                    6
                } else {
                    3
                };
                for scenario in 0..scenarios {
                    let sample = scenario % 3;
                    let color = [0.6, 0.4, 0.2, 0.8];
                    let second = [0.4, 0.6, 0.8, 1.];
                    let animation = [0., 0.25, 0.75][sample];
                    let time = animation + [0.03125, 0.125, 1.][sample];
                    let value = [-0.25, 0.5, 1.][sample];
                    let offsets = if family == Family::Flag {
                        [0.25, 0.25, 0.5, 0.5]
                    } else {
                        [0.25, 0., 0.5, 0.]
                    };
                    let uv = [0., 0., 0.5, 1.];
                    let records: Vec<[[f32; 4]; 5]> = (0..count)
                        .map(|n| {
                            let angle = [0.0_f32, 0.5, 1.][sample];
                            let (sn, cs) = angle.sin_cos();
                            [
                                [cs, -sn, sn, cs],
                                [1.; 4],
                                [0.125, -0.125, 1., 1.],
                                [
                                    [0., 0.25, 0.75][sample],
                                    0.75,
                                    ((scenario / 4 + n) % 3 + 1) as f32,
                                    ((scenario + n) % 4) as f32,
                                ],
                                [1., 0., 0., 0.],
                            ]
                        })
                        .collect();
                    let textured = scenario < 3;
                    let kind = match family {
                        Family::Sprite if animated => {
                            gui::PartKind::Animated(Box::new(gui::Animated {
                                sprite: b"sprite".to_vec(),
                                geometry: geometry(),
                                uv,
                                offsets,
                                effect: base.source.effect.clone(),
                                technique: technique_name,
                                records: records
                                    .iter()
                                    .enumerate()
                                    .map(|(n, &constants)| gui::animation::AnimationDraw {
                                        mask: 1 + n * 2,
                                        texture: 2 + n * 2,
                                        constants,
                                        typed: true,
                                    })
                                    .collect(),
                                masked,
                            }))
                        }
                        Family::Sprite => gui::PartKind::Sprite {
                            sprite: b"sprite".to_vec(),
                            geometry: geometry(),
                            uv,
                            offsets,
                            masked,
                        },
                        Family::Progress => gui::PartKind::Progress {
                            sprite: b"progress".to_vec(),
                            geometry: geometry(),
                            uv: [[0., 0.], [0.5, 0.], [0.5, 1.], [0., 1.]],
                            state: value,
                            colors: [color, second],
                            textured,
                        },
                        Family::Text => gui::PartKind::Text {
                            font: b"font".to_vec(),
                            glyphs: vec![gui::DrawnGlyph {
                                ch: 'A',
                                at: [0., 0.],
                                color,
                                texture: 0,
                                size: [16, 16],
                                uv,
                            }],
                            scale: 1.,
                            modulate: second,
                            border_color: 0,
                            projection: [1.; 2],
                            font_viewport: None,
                        },
                        Family::Flag => gui::PartKind::MaskedFlag {
                            sprite: b"flag".to_vec(),
                            flag: 0,
                            flag_coords: offsets,
                            rect: [0., 0., 16., 16.],
                            native_origin: [0., 16.],
                        },
                        Family::Circular => gui::PartKind::Fan {
                            sprite: b"fan".to_vec(),
                            centre: [8., 8.],
                            radius: [8., 8.],
                            rotation: [0., 0.5, -0.7][sample],
                            weights: [[100, 0], [50, 50], [0, 100]][sample],
                        },
                    };
                    let mut part = part(kind, requested.clone(), technique_name);
                    part.color = color;
                    part.shading.since = time - animation;
                    part.shading.time = time;
                    part.shading.animation = animation;
                    let mut batch = sprites::Batch::default();
                    draw::emit(&part, [0, 0], &Sheets, &mut batch).unwrap();
                    let (actual, refused, _) =
                        sprites::oracle::render_images(&device, &queue, &batch, &pages);
                    assert!(
                        refused.is_empty(),
                        "{file}:{} {defines:?}: {refused:?}",
                        technique.name
                    );

                    let mut constants;
                    let vertices;
                    let declaration;
                    let stride;
                    let known;
                    let mut textures = vec![(0, 0)];
                    let topology;
                    match family {
                        Family::Sprite => {
                            constants = matrix([16, 16], [0., -16., -1.]);
                            constants.extend(color);
                            constants.extend(offsets);
                            constants.extend([time, animation]);
                            constants.resize(64, 0.);
                            vertices = rectangle([0., 0., 16., 16.], uv, &[3, 0, 2, 1], 1., None);
                            declaration = vertices::Declaration::Sprite;
                            stride = 20;
                            known = 104;
                            topology = wgpu::PrimitiveTopology::TriangleStrip;
                            if masked {
                                textures.push((5, 5));
                            }
                            if active_animation {
                                for slot in 1..=4 {
                                    textures.push((
                                        slot,
                                        if slot as usize <= count * 2 {
                                            slot as usize
                                        } else {
                                            6
                                        },
                                    ));
                                }
                            }
                        }
                        Family::Progress => {
                            constants = matrix([16, 16], [0., -16., -1.]);
                            constants.extend(color);
                            constants.extend(second);
                            constants.push(value);
                            constants.resize(28, 0.);
                            vertices = rectangle([0., 0., 16., 16.], uv, &[3, 0, 2, 1], 1., None);
                            declaration = vertices::Declaration::Sprite;
                            stride = 20;
                            known = 100;
                            topology = wgpu::PrimitiveTopology::TriangleStrip;
                            textures = if textured {
                                vec![(0, 0), (1, 1)]
                            } else {
                                vec![(0, 6), (1, 6)]
                            };
                        }
                        Family::Text => {
                            constants = matrix([16, 16], [0.; 3]);
                            constants.extend(second);
                            vertices = rectangle(
                                [0., -16., 16., 16.],
                                uv,
                                &[3, 0, 2, 1, 2, 0],
                                0.,
                                Some((color, 0)),
                            );
                            declaration = vertices::Declaration::Text;
                            stride = 28;
                            known = 80;
                            topology = wgpu::PrimitiveTopology::TriangleList;
                        }
                        Family::Flag => {
                            constants = matrix([16, 16], [0., -16., -1.]);
                            constants.extend(offsets);
                            constants.extend([0.; 4]);
                            vertices = rectangle(
                                [0., 0., 16., 16.],
                                [0., 0., 1., 1.],
                                &[3, 0, 2, 1],
                                1.,
                                None,
                            );
                            declaration = vertices::Declaration::Sprite;
                            stride = 20;
                            known = 96;
                            topology = wgpu::PrimitiveTopology::TriangleStrip;
                            textures.push((1, 1));
                        }
                        Family::Circular => {
                            constants = matrix([16, 16], [8., -8., 0.]);
                            vertices = fan_vertices(
                                [8., 8.],
                                [0., 0.5, -0.7][sample],
                                [[100, 0], [50, 50], [0, 100]][sample],
                            );
                            declaration = vertices::Declaration::Circular;
                            stride = 24;
                            known = 64;
                            topology = wgpu::PrimitiveTopology::TriangleList;
                            textures.push((1, 1));
                        }
                    }
                    let mut direct = inputs(
                        selected_reference,
                        technique_name,
                        vertices,
                        declaration,
                        stride,
                        constants,
                        known,
                        matches!(family, Family::Sprite | Family::Progress),
                        &textures,
                        &views,
                        topology,
                        wgpu::TextureFormat::Rgba8Unorm,
                    );
                    if active_animation {
                        animation_inputs(&mut direct, &records);
                    }
                    arena.reset();
                    let oracle = sprites::oracle::Oracle::prepare(
                        &device,
                        &arena,
                        selected_reference,
                        technique_name,
                        direct,
                    )
                    .unwrap();
                    arena.flush(&queue);
                    let expected = render_with(&device, &queue, |pass| oracle.draw(pass));
                    assert!(
                        expected.iter().any(|&value| value != 0),
                        "{file}:{} {defines:?} scenario={scenario}: the reference drew nothing",
                        technique.name
                    );
                    let different = actual
                        .chunks_exact(4)
                        .zip(expected.chunks_exact(4))
                        .filter(|(a, b)| a != b)
                        .count();
                    let max_error = actual
                        .iter()
                        .zip(&expected)
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap();
                    compared += 1;
                    failures += usize::from(different != 0);
                    let row = format!(
                        "{file}:{} {defines:?} scenario={scenario}: {different}/256 pixels differ, max channel delta={max_error}",
                        technique.name
                    );
                    eprintln!("{row}");
                    rows.push(row);
                    if different != 0
                        && let Some(report) = std::env::var_os("BATTLEPLAN_SHADER_REPORT").or_else(|| std::env::var_os("HOI4_SHADER_REPORT"))
                    {
                        let prefix =
                            format!("{name}-{}-{}-{scenario}", technique.name, defines.join("-"));
                        std::fs::write(
                            Path::new(&report).join(format!("{prefix}-production.rgba")),
                            actual,
                        )
                        .unwrap();
                        std::fs::write(
                            Path::new(&report).join(format!("{prefix}-reference.rgba")),
                            expected,
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
    for row in &rows {
        println!("{row}");
    }
    println!("SUBMISSION COMPARISON: {compared} renders, {failures} with differing pixels");
    if let Some(report) = std::env::var_os("BATTLEPLAN_SHADER_REPORT").or_else(|| std::env::var_os("HOI4_SHADER_REPORT")) {
        std::fs::write(
            Path::new(&report).join("submission-comparison.txt"),
            rows.join("\n"),
        )
        .unwrap();
    }
    assert!(compared > 0);
    assert_eq!(failures, 0, "see the per-effect pixel report");
}

pub(super) fn fan_vertices(radius: [f32; 2], rotation: f32, weights: [i32; 2]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut triangle = 0;
    let mut first = [rotation.cos(), rotation.sin()];
    for (sector, count) in weights.into_iter().enumerate() {
        for _ in 0..count.max(0) {
            triangle += 1;
            let angle = triangle as f32 * std::f32::consts::TAU / 100. + rotation;
            let last = [angle.cos(), angle.sin()];
            for point in [first, [0.; 2], last] {
                for value in [
                    point[0] * radius[0],
                    point[1] * radius[1],
                    point[0],
                    point[1],
                    sector as f32,
                    0.,
                ] {
                    bytes.extend(value.to_le_bytes());
                }
            }
            first = last;
        }
    }
    bytes
}

fn texture(seed: u32) -> dds::Texture {
    dds::Texture {
        format: dds::TextureFormat::Rgba8,
        width: 16,
        height: 16,
        faces: 1,
        mips: vec![
            (0..5)
                .map(|mip| {
                    let size = 16 >> mip;
                    dds::MipLevel {
                        width: size,
                        height: size,
                        data: ((0..size * size)
                            .flat_map(|p| {
                                [
                                    ((p % size * 17 + seed * 23) % 256) as u8,
                                    ((p / size * 13 + seed * 17) % 256) as u8,
                                    (mip * 40 + seed * 19) as u8,
                                    (64 + (p + seed * 11) % 192) as u8,
                                ]
                            })
                            .collect::<Vec<_>>()).into(),
                    }
                })
                .collect(),
        ],
    }
}
