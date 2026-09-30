use super::*;
use crate::map::{native_effects as model, sprites};
use eframe::wgpu;
use sha2::{Digest, Sha256};

#[test]
#[ignore = "requires installed GUI effects, compiler and GPU; set BATTLE_PLAN_GAME_DIR"]
fn compare_installed_native_models() {
    let root = PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("BATTLE_PLAN_GAME_DIR"));
    if let Some(report) = std::env::var_os("BATTLE_PLAN_SHADER_REPORT").or_else(|| std::env::var_os("HOI4_SHADER_REPORT")) {
        std::fs::create_dir_all(report).unwrap();
    }
    let compiler = Compiler::discover().unwrap();
    let (_serial, device, queue) = device();
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let textures: Vec<_> = [0, 1, 5, 2, 3, 4]
        .map(|seed| texture(&device, &queue, seed))
        .into();
    let views: Vec<_> = textures
        .iter()
        .map(|t| t.create_view(&Default::default()))
        .collect();
    let bound = BTreeMap::from([
        (binding(0, Stage::Pixel), views[0].clone()),
        (binding(1, Stage::Pixel), views[1].clone()),
        (binding(5, Stage::Pixel), views[2].clone()),
        (binding(2, Stage::Pixel), views[3].clone()),
        (binding(3, Stage::Pixel), views[4].clone()),
        (binding(4, Stage::Pixel), views[5].clone()),
    ]);
    let mut rows = Vec::new();
    let mut failures = 0;
    let mut compared = 0;
    for file in std::fs::read_dir(root.join("gfx/FX")).unwrap() {
        let path = file.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if !is_gui_effect(&name) {
            continue;
        }
        let file = format!("gfx/FX/{name}");
        let program = installed_content::gui::native::shader::program::read_with(file.as_bytes(), &|p| {
            std::fs::read(root.join(String::from_utf8_lossy(p).as_ref())).ok()
        })
        .unwrap();
        let facts = model::EffectFacts {
            file: file.clone(),
            identity: Sha256::digest(std::fs::read(&path).unwrap()).into(),
            includes: program
                .dependencies
                .iter()
                .filter(|p| {
                    p.as_slice() != file.as_bytes()
                        && p.as_slice() != program.header.path.as_bytes()
                })
                .map(|p| {
                    let p = String::from_utf8_lossy(p).into_owned();
                    (
                        p.clone(),
                        std::fs::read(root.join(&p))
                            .ok()
                            .map(|b| Sha256::digest(b).into()),
                    )
                })
                .collect(),
        };
        let samplers = std::array::from_fn(|slot| program.sampler(slot as u32).map(sampler));
        for defines in variants(&program) {
            let animated = defines.iter().any(|d| d == "ANIMATED");
            let count = if defines.iter().any(|d| d == "NUM_ANIMATIONS_2") {
                2
            } else {
                1
            };
            let effect =
                match model::resolve(&facts, samplers, defines.iter().any(|d| d == "MASKING")) {
                    Ok(effect) => effect,
                    Err(reason) => {
                        rows.push(format!("UNMODELLED {file} {defines:?}: {reason}"));
                        continue;
                    }
                };
            for technique in &program.techniques {
                let model::Technique::Run { pixel, blend: _ } = effect.technique(&technique.name)
                else {
                    rows.push(format!("UNMODELLED {file}:{} {defines:?}", technique.name));
                    continue;
                };
                let compiled = compiler.compile(&program, technique, &defines).unwrap();
                let state = gpu::State::authored(&program, technique).unwrap();
                let pipeline = gpu::Pipeline::with_state(
                    &device,
                    &compiled,
                    &program,
                    technique,
                    wgpu::TextureFormat::Rgba8Unorm,
                    1,
                    &state,
                )
                .unwrap();
                for scenario in 0..if animated { 12 } else { 3 } {
                    let sample_case = scenario % 3;
                    let records: Vec<[[f32; 4]; 5]> = (0..count)
                        .map(|n| {
                            let angle = [0.0_f32, 0.5, 1.0][sample_case];
                            let (sn, cs) = angle.sin_cos();
                            [
                                [cs, -sn, sn, cs],
                                [1., 1., 1., 1.],
                                [0.125, -0.125, 1., 1.],
                                [
                                    [0., 0.25, 0.75][sample_case],
                                    0.75,
                                    ((scenario / 4 + n) % 3 + 1) as f32,
                                    ((scenario + n) % 4) as f32,
                                ],
                                [1., 0., 0., 0.],
                            ]
                        })
                        .collect();
                    let mut b1 = records.concat().concat();
                    b1.resize(40, 0.);
                    let color = [0.6_f32, 0.4, 0.2, 0.8];
                    let second = [0.4_f32, 0.6, 0.8, 1.0];
                    let animation = [0., 0.25, 0.75][sample_case];
                    let time = animation + [0.03125, 0.125, 1.][sample_case];
                    let offsets = if matches!(pixel, model::Pixel::MaskedFlag(_)) {
                        [0.25, 0.25, 0.5, 0.5]
                    } else {
                        [0.25, 0., 0.5, 0.]
                    };
                    let value = [-0.25, 0.5, 1.0][sample_case];
                    let corners = [[-1_f32, 1.], [1., 1.], [1., -1.], [-1., -1.]];
                    let uv = if pixel == model::Pixel::Circular {
                        [[-1., 1.], [1., 1.], [1., -1.], [-1., -1.]]
                    } else {
                        [[0_f32, 0.], [0.5, 0.], [0.5, 1.], [0., 1.]]
                    };
                    let mut constants = vec![0_f32; 28];
                    for i in [0, 5, 10, 15] {
                        constants[i] = 1.;
                    }
                    match pixel {
                        model::Pixel::Circular => {}
                        model::Pixel::Text => constants[16..20].copy_from_slice(&second),
                        model::Pixel::MaskedFlag(_) => constants[16..20].copy_from_slice(&offsets),
                        model::Pixel::Progress { .. } => {
                            constants[16..20].copy_from_slice(&color);
                            constants[20..24].copy_from_slice(&second);
                            constants[24] = value;
                        }
                        model::Pixel::Sprite(_) => {
                            constants[16..20].copy_from_slice(&color);
                            constants[20..24].copy_from_slice(&offsets);
                            constants[24] = time;
                            constants[25] = animation;
                        }
                    }
                    let mut bytes = Vec::new();
                    for i in [0, 1, 2, 0, 2, 3] {
                        if pixel == model::Pixel::Circular {
                            bytes.extend_from_slice(bytemuck::cast_slice(&[
                                corners[i][0],
                                corners[i][1],
                                uv[i][0],
                                uv[i][1],
                                value,
                                0.,
                            ]));
                        } else {
                            bytes.extend_from_slice(bytemuck::cast_slice(&[
                                corners[i][0],
                                corners[i][1],
                                1.,
                                uv[i][0],
                                uv[i][1],
                            ]));
                        }
                        if pixel == model::Pixel::Text {
                            bytes.extend_from_slice(&[51, 102, 153, 204, 0, 0, 0, 0]);
                        }
                    }
                    let text = pixel == model::Pixel::Text;
                    arena.reset();
                    let draw = pipeline
                        .prepare(
                            &arena,
                            &BTreeMap::from([
                                (0, bytemuck::cast_slice(&constants).to_vec()),
                                (1, bytemuck::cast_slice(&constants).to_vec()),
                                (
                                    binding(1, Stage::Vertex),
                                    bytemuck::cast_slice(&b1).to_vec(),
                                ),
                                (binding(1, Stage::Pixel), bytemuck::cast_slice(&b1).to_vec()),
                            ]),
                            &bound,
                            &vertices::Vertices {
                                declaration: if pixel == model::Pixel::Circular {
                                    vertices::Declaration::Circular
                                } else if text {
                                    vertices::Declaration::Text
                                } else {
                                    vertices::Declaration::Sprite
                                },
                                bytes: &bytes,
                                stride: if text {
                                    28
                                } else if pixel == model::Pixel::Circular {
                                    24
                                } else {
                                    20
                                },
                                count: 6,
                            },
                        )
                        .unwrap();
                    arena.flush(&queue);
                    let actual = render(&device, &queue, &pipeline, &draw);
                    let quad = sprites::Quad {
                        texture: 0,
                        rect: [0.; 4],
                        uv: [0.; 4],
                        color,
                        rotation: 0.,
                        layer: 0,
                        shade: 0.,
                        exact: Some(sprites::Exact {
                            native: None,
                            corners,
                            uv,
                            scissor: None,
                            pixel: pixel.packed(),
                            blend: model::Blend::Off,
                            samplers: [0, 1, 5].iter().enumerate().fold(0, |packed, (i, slot)| {
                                packed | samplers[*slot].map_or(0, |s| s.packed()) << (i * 10)
                            }),
                            lod_bias: [0, 1, 5]
                                .map(|slot| samplers[slot].map_or(0., |s| s.lod_bias)),
                            second: Some(1),
                            masking: Some(5),
                            offsets,
                            inputs: [time - animation, animation, value, 0.],
                            second_color: second,
                        }),
                    };
                    let apply = animated
                        && !(name == "buttonstate_onlydisable.shader"
                            && technique.name == "Disable");
                    let source = if apply {
                        sprites::oracle::hardware_source(&sprites::oracle::animated_source(
                            &records, true,
                        ))
                    } else {
                        sprites::EXACT_SHADER.to_owned()
                    };
                    let oracle_views = views[..if apply {
                        if count == 2 { 6 } else { 4 }
                    } else {
                        3
                    }]
                        .iter()
                        .collect::<Vec<_>>();
                    let oracle = sprites::oracle::Oracle::with_source(
                        &device,
                        quad,
                        &oracle_views,
                        state.blend,
                        state.write_mask,
                        &source,
                    );
                    let oracle = if apply {
                        oracle.animation(&device, &records)
                    } else {
                        oracle
                    };
                    let expected = render_with(&device, &queue, |pass| oracle.draw(pass));
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
                    eprintln!(
                        "{file}:{} {defines:?} scenario={scenario}: {different}/256 differ, delta={max_error}",
                        technique.name
                    );
                    rows.push(format!("{file}:{} {defines:?} scenario={scenario}: {different}/256 pixels differ, max channel delta={max_error}", technique.name));
                    if different != 0
                        && let Some(report) = std::env::var_os("BATTLE_PLAN_SHADER_REPORT").or_else(|| std::env::var_os("HOI4_SHADER_REPORT"))
                    {
                        let prefix =
                            format!("{name}-{}-{}-{scenario}", technique.name, defines.join("-"));
                        std::fs::write(
                            Path::new(&report).join(format!("{prefix}-compiled.rgba")),
                            actual,
                        )
                        .unwrap();
                        std::fs::write(
                            Path::new(&report).join(format!("{prefix}-model.rgba")),
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
    println!("MODEL COMPARISON: {compared} renders, {failures} with differing pixels");
    if let Some(report) = std::env::var_os("BATTLE_PLAN_SHADER_REPORT").or_else(|| std::env::var_os("HOI4_SHADER_REPORT")) {
        std::fs::write(
            Path::new(&report).join("model-comparison.txt"),
            rows.join("\n"),
        )
        .unwrap();
    }
    assert!(compared > 0);
    assert_eq!(failures, 0, "see the per-effect pixel report");
}

fn sampler(s: &installed_content::gui::native::shader::program::Sampler) -> model::Sampler {
    use installed_content::gui::native::shader::{Address, program::Filter};
    let address = |a| match a {
        Address::Wrap => model::Address::Wrap,
        Address::Mirror => model::Address::Mirror,
        Address::Clamp => model::Address::Clamp,
        Address::Border => model::Address::Border,
        Address::MirrorOnce => model::Address::MirrorOnce,
    };
    let filter = |f| {
        if matches!(f, Filter::Linear | Filter::Anisotropic) {
            model::Filter::Linear
        } else {
            model::Filter::Point
        }
    };
    model::Sampler {
        u: address(s.u),
        v: address(s.v),
        mag: filter(s.mag),
        min: filter(s.min),
        mip: filter(s.mip),
        lod_bias: s.lod_bias,
    }
}

fn texture(device: &wgpu::Device, queue: &wgpu::Queue, seed: u32) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("model comparison texels"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 5,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for mip in 0..5 {
        let size = 16 >> mip;
        let bytes: Vec<u8> = (0..size * size)
            .flat_map(|p| {
                [
                    ((p % size * 17 + seed * 23) % 256) as u8,
                    ((p / size * 13 + seed * 17) % 256) as u8,
                    (mip * 40 + seed * 19) as u8,
                    (64 + (p + seed * 11) % 192) as u8,
                ]
            })
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
    }
    texture
}
