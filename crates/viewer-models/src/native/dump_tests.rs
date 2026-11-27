use crate::native::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use viewer_gpu::camera::MapDefines;
use viewer_gpu::camera::MapSize;
use viewer_gpu::gpu_contract::SCENE_FORMAT;
use viewer_gpu::test_support::gpu as tests;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hoi4_dump_skin_{}_{nonce}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn draw(
    model: &ModelPass,
    arena: &shader_pipeline::gpu::Arena,
    pass: &mut wgpu::RenderPass<'_>,
    object: &mut ObjectConstants,
) -> io::Result<()> {
    model.record_native(
        arena,
        pass,
        ModelFrame {
            camera: Camera {
                center: [0., 0.],
                height: 100.,
                map: MapSize {
                    width: 100,
                    height: 100,
                },
                defines: MapDefines::test(),
                pose: viewer_gpu::camera::Pose::Map,
            },
            offsets: &[0., 0.],
            visible: None,
            seen: None,
            alpha: 1.,
        },
        Inputs {
            samples: 1,
            frames: &[&[0; 416][..], &[0; 416][..]],
            grid: &[0; 16],
            textures: [&model.textures[0].as_ref().unwrap().view; 10],
        },
        Queue::Regular,
        object,
    )
}

fn fixture() -> (TestDir, ModelLibrary, usize) {
    content_reader_script::test_game::enter();
    let directory = TestDir::new();
    let root = &directory.0;
    std::fs::create_dir_all(root.join("gfx/FX")).unwrap();
    std::fs::write(root.join("gfx/FX/defines_hlsl_dx11.fxh"), []).unwrap();
    std::fs::write(
        root.join("gfx/FX/test.shader"),
        br#"
        VertexStruct I
        {
            float3 position : POSITION;
        };
        VertexStruct O
        {
            float4 position : SV_POSITION;
        };
        ConstantBuffer(3,0) { float4 unavailable;
        };
        MainCode V
        [[
            O main(I v) { O o; o.position = float4(v.position, 1); return o; }
        ]]
        MainCode P
        [[
            float4 main(O v) : SV_TARGET { return float4(0, 1, 0, 1); }
        ]]
        MainCode UnboundV
        [[
            O main(I v) { O o; o.position = float4(v.position, 1) + unavailable; return o; }
        ]]
        RasterizerState Visible { CullMode = none }
        Effect Test { VertexShader = V PixelShader = P RasterizerState = Visible }
        Effect TestSkinned { VertexShader = V PixelShader = P RasterizerState = Visible }
        Effect TestShadow { VertexShader = V PixelShader = P RasterizerState = Visible }
        Effect TestSkinnedShadow { VertexShader = V PixelShader = P RasterizerState = Visible }
        Effect Unbound { VertexShader = UnboundV PixelShader = P RasterizerState = Visible }
        Effect UnboundShadow { VertexShader = UnboundV PixelShader = P RasterizerState = Visible }
        ConstantBuffer(0,0) {
            float4 firstScene;
        };
        ConstantBuffer(1,0) {
            float4 firstObject;
        };
        ConstantBuffer(2,0) {
            float4 firstSkin;
        };
        MainCode ObjectV
        [[
            O main(I v) { O o; o.position = float4(v.position,1) + firstObject * 0.001 + firstScene * 0.0001; return o; }
        ]]
        MainCode SkinV
        [[
            O main(I v) { O o; o.position = float4(v.position,1) + firstObject * 0.001 + firstScene * 0.0001 + firstSkin * 0.0001; return o; }
        ]]
        MainCode ObjectP
        [[
            float4 main(O v) : SV_TARGET { return abs(firstObject) + firstSkin * 0.01; }
        ]]
        BlendState AlphaBlend { BlendEnable = yes SourceBlend = src_alpha DestBlend = inv_src_alpha }
        Effect Object { VertexShader = ObjectV PixelShader = ObjectP RasterizerState = Visible }
        Effect ObjectAlpha { VertexShader = ObjectV PixelShader = ObjectP RasterizerState = Visible BlendState = AlphaBlend }
        Effect ObjectShadow { VertexShader = ObjectV PixelShader = P RasterizerState = Visible }
        Effect ObjectSkinned { VertexShader = SkinV PixelShader = ObjectP RasterizerState = Visible }
        Effect ObjectSkinnedAlpha { VertexShader = SkinV PixelShader = ObjectP RasterizerState = Visible BlendState = AlphaBlend }
        Effect ObjectSkinnedShadow { VertexShader = SkinV PixelShader = P RasterizerState = Visible }
        Effect Alpha { VertexShader = ObjectV PixelShader = ObjectP RasterizerState = Visible BlendState = AlphaBlend }
        Effect AlphaShadow { VertexShader = ObjectV PixelShader = P RasterizerState = Visible }
    "#,
    )
    .unwrap();
    std::fs::write(
        root.join("gfx/FX/kept.shader"),
        br#"
        VertexStruct I
        {
            float3 position : POSITION;
        };
        VertexStruct O
        {
            float4 position : SV_POSITION;
        };
        ConstantBuffer(0,0) {
            float4 firstScene;
        };
        ConstantBuffer(1,0) {
            float4 world0;
            float4 world1;
            float4 world2;
            float4 world3;
        };
        MainCode WorldV
        [[
            O main(I v) { O o; o.position = float4(v.position,1) + (world0 + world1 + world2 + world3) * 0.001 + firstScene * 0.0001; return o; }
        ]]
        MainCode WorldP
        [[
            float4 main(O v) : SV_TARGET { return abs(world0); }
        ]]
        MainCode P
        [[
            float4 main(O v) : SV_TARGET { return float4(0, 1, 0, 1); }
        ]]
        RasterizerState Visible { CullMode = none }
        Effect World { VertexShader = WorldV PixelShader = WorldP RasterizerState = Visible }
        Effect WorldShadow { VertexShader = WorldV PixelShader = P RasterizerState = Visible }
    "#,
    )
    .unwrap();
    let layers = Arc::new(ResolvedLayers::from_folders(&[(root, &[])]).unwrap());
    let mut library = ModelLibrary {
        layers: Some(layers),
        ..Default::default()
    };
    let blank = library.blank(ColorSpace::Linear);
    library.materials.push(ModelMaterial {
        diffuse: blank,
        normal: blank,
        specular: blank,
    });
    for (index, name) in ["unplayed_skin", "defined_triangle"]
        .into_iter()
        .enumerate()
    {
        let skin = index == 0;
        let geometry = Arc::new(mesh_shader::Geometry {
            vertices: [[-0.75, -0.75, 0.], [0.75, -0.75, 0.], [0., 0.75, 0.]]
                .into_iter()
                .flat_map(|position| position.into_iter().chain([0.; 9]))
                .collect(),
            skin: if skin {
                vec![mesh_shader::SkinVertex::from_authored([0; 4], [1., 0., 0., 0.]); 3]
            } else {
                Vec::new()
            },
            indices: vec![0, 1, 2],
            stride: 48,
            uv1: false,
        });
        library.meshes.push(ModelMesh {
            asset: crate::AssetKey::synthetic(&(name)),
            name: name.into(),
            path: format!("{name}.mesh"),
            scale: 1.,
            lod_distances: Vec::new(),
            native: vec![Descriptor {
                name: name.into(),
                node: 0,
                distances: Vec::new(),
                lods: vec![vec![Part {
                    geometry,
                    geometry_key: (name.into(), 1_f32.to_bits(), 0, 0),
                    material_index: 0,
                    material: mesh::Material {
                        shader: Some("Test".into()),
                        shader_file: Some("gfx/FX/test.shader".into()),
                        ..Default::default()
                    },
                }]],
            }],
            radius: 1.,
            height: 1.,
            visual: false,
            own_radius: 1.,
            bound: 1.,
            convoy_radius: None,
            attachments: 0,
            cull_distance_squared: content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
        });
        library.rigs.push(Arc::new(Rig {
            nodes: vec![entity_animation::Node {
                mesh: Arc::default(),
                native: Some(Arc::new(mesh_pose::Source {
                    pdxmesh: true,
                    entity_locators: Vec::new(),
                    binary_locators: Vec::new(),
                    skeleton: if skin {
                        vec![mesh::NativeJoint {
                            name: "joint".into(),
                            parent: 255,
                            inverse_bind: [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.],
                        }]
                    } else {
                        Vec::new()
                    },
                    mesh_scale: 1.,
                    entity_scale: 1.,
                    frozen: None,
                    game_data: None,
                })),
                clips: Vec::new(),
                default: "still".into(),
                follows_parent: false,
                parent: None,
                scale: 1.,
                mesh_scale: 1.,
                skin_base: None,
                rigid: 0,
                locators: BTreeMap::new(),
            }],
            ..Default::default()
        }));
    }
    let diffuse = library.blank(ColorSpace::Srgb);
    let material = library.materials.len();
    library.materials.push(ModelMaterial {
        diffuse,
        normal: blank,
        specular: blank,
    });
    let mut bad_binding = library.meshes[1].clone();
    bad_binding.name = "bad_binding".into();
    bad_binding.native[0].name = bad_binding.name.clone();
    bad_binding.native[0].lods[0][0].material_index = material;
    bad_binding.native[0].lods[0][0].material.shader = Some("Unbound".into());
    library.meshes.push(bad_binding);
    library.rigs.push(library.rigs[1].clone());
    let mut two_part = library.meshes[1].clone();
    two_part.name = "two_part_binding".into();
    two_part.native[0].name = two_part.name.clone();
    let mut unbound = two_part.native[0].lods[0][0].clone();
    unbound.material.shader = Some("Unbound".into());
    two_part.native[0].lods[0].push(unbound);
    library.meshes.push(two_part);
    library.rigs.push(library.rigs[1].clone());
    (directory, library, blank)
}

#[test]
#[ignore = "requires the pinned shader compiler and a GPU"]
fn dump_no_animation_skin_keeps_the_rest_of_the_frame() {
    let (_directory, mut library, blank) = fixture();
    let (_serial, device, queue) = tests::device();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let field = ModelField::build(
        &library,
        (0..library.meshes.len())
            .map(|mesh| {
                (
                    mesh,
                    ModelInstance {
                        scale: 1.,
                        position: if library.meshes[mesh].name == "bad_binding" {
                            [7., 0., 0.]
                        } else {
                            [0.; 3]
                        },
                        ..Default::default()
                    },
                )
            })
            .collect(),
        128.,
    );
    let mut model = ModelPass::new(&library, "unit models");
    model.upload(&device, &queue, &library, field).unwrap();
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let texture = |format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let color = texture(SCENE_FORMAT);
    let depth = texture(DEPTH_FORMAT);
    let color_view = color.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut object = ObjectConstants::default();
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        draw(&model, &arena, &mut pass, &mut object).unwrap();
        assert_eq!(
            model.drawn(),
            2,
            "the defined entity still draws in each wrap view"
        );

        library.meshes[1].native[0].lods[0][0].material.shader = Some("Missing".into());
        let field = ModelField::build(
            &library,
            vec![(
                1,
                ModelInstance {
                    scale: 1.,
                    ..Default::default()
                },
            )],
            128.,
        );
        let mut broken = ModelPass::new(&library, "unit models");
        broken.upload(&device, &queue, &library, field).unwrap();
        let error = draw(&broken, &arena, &mut pass, &mut object).unwrap_err();
        assert!(
            error.to_string().contains("fallback is unavailable"),
            "{error}"
        );
        assert!(broken.refusal_lines(arena.frame()).is_empty());
        assert_eq!(broken.drawn(), 0);
    }
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16 * 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(16),
            },
        },
        color.size(),
    );
    arena.flush(&queue);
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    readback.map_async(wgpu::MapMode::Read, .., move |result| {
        send.send(result).unwrap();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    receive.recv().unwrap().unwrap();
    let pixels = readback.get_mapped_range(..).unwrap();
    assert!(
        pixels.chunks_exact(256).any(|row| row[..128]
            .chunks_exact(8)
            .any(|pixel| u16::from_le_bytes([pixel[2], pixel[3]]) == 0x3c00)),
        "the defined triangle reaches the frame's color target"
    );
    drop(pixels);
    readback.unmap();
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
    assert_eq!(
        f32::from_le_bytes(object.bytes[48..52].try_into().unwrap()),
        0.
    );
    assert_eq!(
        f32::from_le_bytes(object.pixel.unwrap()[48..52].try_into().unwrap()),
        0.,
        "the failed binding keeps the last submitted actor's object constants"
    );
    let refusals = model.refusal_lines(arena.frame());
    assert_eq!(refusals.len(), 3, "{refusals:?}");
    for (name, why) in [
        (
            "unplayed_skin",
            "skin: native skeleton has no played animation",
        ),
        ("bad_binding", "native constant buffer 3 has not been bound"),
        (
            "two_part_binding",
            "native constant buffer 3 has not been bound",
        ),
    ] {
        assert!(
            refusals.iter().any(
                |line| line.contains(&format!("1 refused actor {name}:")) && line.contains(why)
            ),
            "{refusals:?}"
        );
    }

    drop(model);
    let mut oversized = library.textures[blank].source().unwrap();
    oversized.width = device.limits().max_texture_dimension_2d + 1;
    library.textures[blank] = crate::ModelTexture::held(oversized);
    let field = ModelField::build(
        &library,
        vec![(
            1,
            ModelInstance {
                scale: 1.,
                ..Default::default()
            },
        )],
        128.,
    );
    let mut oversized = ModelPass::new(&library, "unit models");
    let error = oversized
        .upload(&device, &queue, &library, field)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(oversized.material_refusals.is_empty());
}

fn expected_draw_instances(frame: usize, kind: Queue) -> Vec<usize> {
    let mut result = Vec::new();
    if frame == 5 || (frame == 4 && kind != Queue::Caster) {
        return result;
    }
    for _ in 0..2 {
        for instance in 0..6 {
            if (instance < 3 && (matches!(frame, 2 | 3) || (frame == 1 && instance == 0)))
                || (frame == 6 && instance == 3)
            {
                continue;
            }
            let parts = match kind {
                Queue::Regular if frame == 2 => 0,
                Queue::Regular => 1,
                Queue::Alternate if frame == 2 => 2,
                Queue::Alternate => usize::from(instance >= 3),
                Queue::Caster => {
                    if instance < 3 {
                        1
                    } else {
                        2
                    }
                }
            };
            result.extend(std::iter::repeat_n(instance, parts));
        }
    }
    result
}

fn expected_fixture_object(instance: usize, payload: Option<[f32; 8]>) -> [u8; 208] {
    let mut result = [0; 208];
    let mut world = mesh_pose::IDENTITY;
    world[3][0] = instance as f32 * 0.1;
    result[..64].copy_from_slice(bytemuck::bytes_of(&world));
    if let Some(payload) = payload {
        result[64..96].copy_from_slice(bytemuck::bytes_of(&payload));
    }
    result
}

fn expected_fixture_refusals(frame: usize, kind: Queue, label: &str) -> Vec<String> {
    let demanded = |mesh| match kind {
        Queue::Regular => !matches!(frame, 2 | 5),
        Queue::Alternate => matches!(frame, 2 | 5) || mesh == 1,
        Queue::Caster => true,
    };
    let mut result = Vec::new();
    let mut add = |name: &str, count, why: &str| {
        result.push(format!(
            "Map shader {label}: {count} refused actor {name}: {why}"
        ));
    };
    match frame {
        1 => add("unplayed_skin", 1, "synthetic actor world failure"),
        2 => add("unplayed_skin", 3, "skin: synthetic skin failure"),
        3 if demanded(0) => add(
            "unplayed_skin",
            3,
            "skin: vertex reads outside 0 initialized bones",
        ),
        6 => add(
            "defined_triangle",
            1,
            "native mesh roll requires the caller's original basis",
        ),
        _ => {}
    }
    for (mesh, name) in [
        "unplayed_skin",
        "defined_triangle",
        "bad_binding",
        "two_part_binding",
    ]
    .into_iter()
    .enumerate()
    {
        if frame == 5 && mesh != 2 {
            add(name, 3, "synthetic material refusal");
        } else if demanded(mesh) {
            if frame == 4 && kind != Queue::Caster {
                add(name, 3, "native mesh material is not resident");
            } else if mesh >= 2 {
                add(name, 3, "native constant buffer 3 has not been bound");
            }
        }
    }
    result.sort_by(|a, b| {
        a.split_once("refused actor ")
            .unwrap()
            .1
            .cmp(b.split_once("refused actor ").unwrap().1)
    });
    result
}

#[test]
#[ignore = "requires the pinned shader compiler and a GPU"]
fn successful_native_programs_keep_multiframe_constants_refusals_and_descriptor_commits() {
    let (_directory, mut library, blank) = fixture();
    for mesh in [0, 1, 3] {
        library.meshes[mesh].native[0].lods[0][0].material.shader = Some("Object".into());
    }
    let mut alternate = library.meshes[1].native[0].lods[0][0].clone();
    alternate.material.shader = Some("Alpha".into());
    library.meshes[1].native[0].lods[0].push(alternate);
    let (_serial, device, queue) = tests::device();
    let field = ModelField::build(
        &library,
        (0..library.meshes.len())
            .flat_map(|mesh| {
                [0_u32, 1, 0].map(move |identity| {
                    (
                        mesh,
                        ModelInstance {
                            scale: 1.,
                            animation_identity: identity,
                            ..Default::default()
                        },
                    )
                })
            })
            .collect(),
        128.,
    );
    let mut model = ModelPass::new(&library, "unit models");
    model.upload(&device, &queue, &library, field).unwrap();
    let arenas = [
        shader_pipeline::gpu::Arena::new(&device),
        shader_pipeline::gpu::Arena::new(&device),
    ];
    let mut objects = [ObjectConstants::default(), ObjectConstants::default()];
    let mut globals = model.textures[blank].as_ref().unwrap().view.clone();
    let camera = Camera {
        center: [0., 0.],
        height: 100.,
        map: MapSize {
            width: 100,
            height: 100,
        },
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    };
    let mut expected_pixel = None;
    for frame in 0..8 {
        model.label = ["unit models", "city sprawl", "supply trains"][frame % 3];
        if frame == 3 {
            globals = model.textures[blank]
                .as_ref()
                .unwrap()
                .texture
                .create_view(&Default::default());
        }
        let absent = if frame == 4 {
            model.textures[blank].take()
        } else {
            None
        };
        if frame == 5 {
            model
                .material_refusals
                .insert(0, "synthetic material refusal".into());
        }
        if frame == 6 {
            model.field.instances[3].roll = 1.;
        }
        let mut scene = [0; 416];
        for (index, value) in [0_u32, 0x80000000, 0x7fc01234, 0xffc05678]
            .into_iter()
            .enumerate()
        {
            scene[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        scene[32] = frame as u8;
        *model.poses.borrow_mut() = (
            arenas[0].frame(),
            (0..model.field.instances.len())
                .map(|index| {
                    let mut world = mesh_pose::IDENTITY;
                    world[3][0] = index as f32 * 0.1;
                    let mut bone = mesh_pose::IDENTITY;
                    bone[0][0] = f32::from_bits(if frame % 2 == 0 {
                        0x80000000
                    } else {
                        0x7fc01234
                    });
                    bone[3][0] = (index + frame) as f32;
                    Some(Arc::from([mesh_pose::Actor {
                        world: if frame == 1 && index == 0 {
                            Err("synthetic actor world failure".into())
                        } else {
                            Ok(world)
                        },
                        pose: if frame == 2 {
                            Err("synthetic skin failure".into())
                        } else {
                            Ok(mesh_pose::Pose {
                                bones: if frame == 3 { Vec::new() } else { vec![bone] },
                                joints: Vec::new(),
                            })
                        },
                    }]))
                })
                .collect(),
        );
        let frames = [scene.as_slice(), scene.as_slice()];
        let inputs = Inputs {
            samples: 1,
            frames: &frames,
            grid: &[frame as u8; 16],
            textures: [&globals; 10],
        };
        let model_frame = ModelFrame {
            camera,
            offsets: &[0., 0.5],
            visible: None,
            seen: None,
            alpha: 0.5,
        };
        for kind in [
            Queue::Regular,
            Queue::Alternate,
            Queue::Caster,
            Queue::Regular,
        ] {
            for object in &mut objects {
                object.begin();
            }
            let invoke =
                |cold: bool, arena: &shader_pipeline::gpu::Arena, object: &mut ObjectConstants| {
                    if cold && let Some(native) = model.native.borrow_mut().as_mut() {
                        for resident in &mut native.residents {
                            resident.program = None;
                        }
                    }
                    model.drawn.set(0);
                    *model.refusals.borrow_mut() = (arena.frame(), BTreeSet::new());
                    let result = model.prepare_native(arena, model_frame, inputs, kind, object);
                    (result, model.drawn(), model.refusal_lines(arena.frame()))
                };
            let (cold, cold_count, cold_refusals) = invoke(true, &arenas[0], &mut objects[0]);
            let (warm, warm_count, warm_refusals) = invoke(false, &arenas[1], &mut objects[1]);
            let (cold, warm) = (cold.unwrap(), warm.unwrap());
            let expected = expected_draw_instances(frame, kind);
            let count = match kind {
                Queue::Regular => [12, 10, 0, 6, 0, 0, 10, 12][frame],
                Queue::Alternate => [6, 6, 12, 6, 0, 0, 4, 6][frame],
                Queue::Caster => [18, 16, 12, 12, 18, 0, 14, 18][frame],
            };
            assert_eq!(expected.len(), count);
            assert_eq!(warm.draws.len(), count, "frame {frame}");
            assert_eq!(
                warm_count as usize,
                if kind == Queue::Caster { 0 } else { count }
            );
            assert_eq!(
                warm_refusals,
                expected_fixture_refusals(frame, kind, model.label)
            );
            let payload = if kind == Queue::Caster || frame % 3 == 1 {
                None
            } else if frame % 3 == 0 {
                Some([0., 0., 1., 1., 1., 0., 0., 0.])
            } else {
                Some([1., 1., 1., 0.5, 0., 0., 0., 0.])
            };
            let pixel_before = expected_pixel;
            let scratch = if expected.is_empty() {
                [0; 208]
            } else {
                expected_fixture_object(5, payload)
            };
            if !expected.is_empty() && payload.is_some() {
                expected_pixel = Some(scratch);
            }
            assert_eq!(objects[1].bytes, scratch, "frame {frame}");
            assert_eq!(objects[1].pixel, expected_pixel, "frame {frame}");
            for (draw, instance) in warm.draws.iter().zip(expected) {
                let skinned = instance < 3;
                assert_eq!(draw.reference, 0);
                assert_eq!(
                    (
                        draw.vertices,
                        draw.indices,
                        draw.first,
                        draw.count,
                        draw.base
                    ),
                    (((48, skinned), 0), 0, if skinned { 0 } else { 3 }, 3, 0)
                );
                let object = expected_fixture_object(instance, payload);
                let pixel = if payload.is_some() {
                    Some(object)
                } else {
                    pixel_before
                };
                let mut bone = mesh_pose::IDENTITY;
                bone[0][0] = f32::from_bits(if frame % 2 == 0 {
                    0x80000000
                } else {
                    0x7fc01234
                });
                bone[3][0] = (instance + frame) as f32;
                let (proof, offsets) = draw.bindings.preparation_proof();
                let mut slots: Vec<_> = proof.iter().map(|(slot, _)| *slot).collect();
                slots.sort();
                let mut expected_slots = if kind == Queue::Caster {
                    vec![0, 2]
                } else {
                    vec![0, 2, 3, 5]
                };
                if skinned {
                    expected_slots.push(4);
                }
                expected_slots.sort();
                assert_eq!(slots, expected_slots);
                assert_eq!(offsets.len(), slots.len());
                for (slot, bytes) in proof {
                    let expected: &[u8] = match *slot {
                        0 => &scene,
                        2 => &object,
                        3 => pixel.as_ref().unwrap(),
                        4 => bytemuck::bytes_of(&bone),
                        5 => inputs.grid,
                        _ => panic!("unexpected fixture binding {slot}"),
                    };
                    assert_eq!(
                        bytes, expected,
                        "frame {frame}, instance {instance}, binding {slot}"
                    );
                }
            }
            assert_eq!(cold_count, warm_count);
            assert_eq!(cold_refusals, warm_refusals);
            assert_eq!(arenas[0].refusals(), arenas[1].refusals());
            assert_eq!(objects[0].bytes, objects[1].bytes);
            assert_eq!(objects[0].pixel, objects[1].pixel);
            assert!(
                objects[0]
                    .retained
                    .stream
                    .same_bindings(&objects[1].retained.stream)
            );
            assert_eq!(cold.draws.len(), warm.draws.len());
            for (a, b) in cold.draws.iter().zip(&warm.draws) {
                assert_eq!(
                    (a.reference, a.vertices, a.indices, a.first, a.count, a.base),
                    (b.reference, b.vertices, b.indices, b.first, b.count, b.base)
                );
                assert_eq!(
                    a.bindings.preparation_proof(),
                    b.bindings.preparation_proof(),
                    "frame {frame}"
                );
                assert!(a.bindings.same_resources(&b.bindings));
            }
            let native = model.native.borrow();
            let native = native.as_ref().unwrap();
            if let Some(lods) = native.slots.get(3).and_then(|nodes| nodes.first()) {
                for id in lods
                    .iter()
                    .flatten()
                    .flat_map(|variants| variants.iter().flatten())
                {
                    assert!(native.residents[*id].program.is_none());
                }
            }
        }
        if let Some(texture) = absent {
            model.textures[blank] = Some(texture);
        }
        model.material_refusals.remove(&0);
        model.field.instances[3].roll = 0.;
        for arena in &arenas {
            arena.flush(&queue);
            arena.reset();
        }
    }

    let mut missing = library.meshes[1].clone();
    missing.name = "missing_shared".into();
    missing.native[0].lods[0][0].material.shader = Some("Missing".into());
    library.meshes.push(missing);
    library.rigs.push(library.rigs[1].clone());
    let field = ModelField::build(
        &library,
        [1, library.meshes.len() - 1]
            .map(|mesh| {
                (
                    mesh,
                    ModelInstance {
                        scale: 1.,
                        ..Default::default()
                    },
                )
            })
            .to_vec(),
        128.,
    );
    model.upload(&device, &queue, &library, field).unwrap();
    model.label = "unit models";
    let frames = [&[0; 416][..]];
    let inputs = Inputs {
        samples: 1,
        frames: &frames,
        grid: &[0; 16],
        textures: [&globals; 10],
    };
    let frame = ModelFrame {
        camera,
        offsets: &[0.],
        visible: None,
        seen: None,
        alpha: 1.,
    };
    let mut errors = Vec::new();
    for (index, object) in objects.iter_mut().enumerate() {
        let arena = &arenas[index];
        model.drawn.set(0);
        let error = model.prepare_native(arena, frame, inputs, Queue::Regular, object)
        .err()
        .unwrap();
        errors.push(error.to_string());
        assert_eq!(model.drawn(), 0);
    }
    assert_eq!(errors[0], errors[1]);
    assert_eq!(errors[0], "native PdxMeshColor fallback is unavailable");
    let scratch = expected_fixture_object(0, Some([0., 0., 1., 1., 1., 0., 0., 0.]));
    assert_eq!(objects[1].bytes, scratch);
    assert_eq!(objects[1].pixel, Some(scratch));
    assert_eq!(objects[0].bytes, objects[1].bytes);
    assert_eq!(objects[0].pixel, objects[1].pixel);
    assert!(
        objects[0]
            .retained
            .stream
            .same_bindings(&objects[1].retained.stream)
    );
}

#[path = "dump_tests/independent.rs"]
mod independent;

#[test]
#[ignore = "requires the pinned shader compiler and a GPU"]
fn native_routes_and_program_cache_keep_multiframe_pixels_arena_state_and_refusals() {
    use viewer_gpu::gpu_contract::read_texture;
    let (_directory, mut library, blank) = fixture();
    library.meshes[1].native[0].lods[0][0].material.shader = Some("Object".into());
    let mut alternate = library.meshes[1].native[0].lods[0][0].clone();
    alternate.material.shader = Some("Alpha".into());
    library.meshes[1].native[0].lods[0].push(alternate);
    library.meshes[2].visual = true;
    library.alternatives.insert(2, 1);
    let (_serial, device, queue) = tests::device();
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let camera = Camera {
        center: [0., 0.],
        height: 100.,
        map: MapSize {
            width: 100,
            height: 100,
        },
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    };
    let render = |draws: &DrawList, caster: bool, samples| {
        let mut encoder = device.create_command_encoder(&Default::default());
        if caster {
            let target = |format| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("native walk caster proof"),
                    size: wgpu::Extent3d {
                        width: 16,
                        height: 16,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            };
            let colour = target(DEPTH_MAP_FORMAT);
            let depth = target(DEPTH_FORMAT);
            let colour_view = colour.create_view(&Default::default());
            let depth_view = depth.create_view(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &colour_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0),
                            store: wgpu::StoreOp::Store,
                        }),
                    }),
                    ..Default::default()
                });
                draws.record(&mut pass);
            }
            queue.submit([encoder.finish()]);
            read_texture(&device, &queue, &colour, 4).unwrap()
        } else {
            let targets = Targets::new(&device, [16, 16], [16, 16], samples, viewer_gpu::gpu_contract::SeaBed::Half);
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(
                        targets.scene_attachment(wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
                    )],
                    depth_stencil_attachment: Some(
                        targets.depth_attachment(wgpu::LoadOp::Clear(1.)),
                    ),
                    ..Default::default()
                });
                draws.record(&mut pass);
            }
            targets.resolve_scene(&mut encoder);
            queue.submit([encoder.finish()]);
            read_texture(&device, &queue, targets.scene_texture(), 8).unwrap()
        }
    };
    let mut nonempty = 0;
    let mut refusals = 0;
    let mut seen = [false; 5];
    for samples in [1, 4] {
        let mut models = [0, 1].map(|_| {
            let mut model = ModelPass::new(&library, "unit models");
            model.upload_library(&device, &queue, &library).unwrap();
            model
        });
        let mut resources = independent::Resources::default();
        let arenas = [
            shader_pipeline::gpu::Arena::new(&device),
            shader_pipeline::gpu::Arena::new(&device),
        ];
        let mut objects = [ObjectConstants::default(), ObjectConstants::default()];
        let mut globals = models.each_ref().map(|model| model.textures[blank].as_ref().unwrap().view.clone());
        for frame in 0..48usize {
            let placed: Vec<_> = (0..16)
                .map(|index| {
                    (
                        index % library.meshes.len(),
                        ModelInstance {
                            scale: 0.25 + (index % 5) as f32 * 0.125,
                            position: [(index % 3) as f32, 0., (index % 2) as f32],
                            animation_seed: index as u32 % 3,
                            animation_identity: index as u32 % 2,
                            animation: if index % 2 == 0 {
                                entity_animation::STOPPED
                            } else {
                                0
                            },
                            roll: if frame % 11 == 1 && index == 5 {
                                0.5
                            } else {
                                0.
                            },
                            ..Default::default()
                        },
                    )
                })
                .collect();
            let mut absent = [None, None];
            for (arm, model) in models.iter_mut().enumerate() {
                model.fall_back();
                model.label = ["unit models", "city sprawl", "supply trains", "unknown caller"][frame % 4];
                if frame % 3 == 0 {
                    let mut field = ModelField::build(&library, placed.clone(), 128.);
                    field.native_order.reverse();
                    let duplicate = field.native_order[0];
                    field.native_order.insert(3, duplicate);
                    model.replace_field(&library, &mut field);
                } else {
                    model.update_placed(placed.clone(), 128.);
                }
                model.set_hidden(if frame % 7 == 2 {
                    BTreeSet::from([(1, 2, 1)])
                } else {
                    BTreeSet::new()
                });
                if arm == 0 && frame % 3 == 0 { model.native_walk.installed = false; }
                if frame % 13 == 4 {
                    globals[arm] = model.textures[blank].as_ref().unwrap().texture.create_view(&Default::default());
                    seen[0] = true;
                }
                if frame % 17 == 6 {
                    absent[arm] = model.textures[blank].take();
                }
                if frame % 19 == 9 {
                    model.material_refusals.insert(0, "injected source-order material refusal".into());
                    seen[2] = true;
                }
            }
            seen[1] |= absent.iter().any(Option::is_some);
            let scene = [0; 416];
            let frames = [scene.as_slice(), scene.as_slice()];
            let grid = [0.125_f32, 0.25, 0.5, 50.];
            let model_frame = ModelFrame {
                camera,
                offsets: &[0., 0.5],
                visible: (frame % 5 == 1).then_some([-2., -2., 3., 3.]),
                seen: (frame % 5 == 1).then_some([-2., -2., 3., 3.]),
                alpha: 0.75,
            };
            for kind in [Queue::Regular, Queue::Alternate, Queue::Caster] {
                let mut outputs = [None, None];
                let mut reports = Vec::new();
                for arm in if frame % 2 == 0 { [0, 1] } else { [1, 0] } {
                    let model = &models[arm];
                    let inputs = Inputs {
                        samples, frames: &frames, grid: bytemuck::cast_slice(&grid),
                        textures: [&globals[arm]; 10],
                    };
                    model.drawn.set(0);
                    *model.refusals.borrow_mut() = (arenas[arm].frame(), BTreeSet::new());
                    objects[arm].begin();
                    if arm == 0 && let Some(native) = model.native.borrow_mut().as_mut() {
                        for resident in &mut native.residents {
                            if let Some((_, program)) = &mut resident.program {
                                program.omit_recipe_for_tests();
                            }
                        }
                    }
                    let result = model.prepare_native(
                        &arenas[arm], model_frame, inputs, kind, &mut objects[arm],
                    );
                    let draws = result.unwrap();
                    refusals += model.refusal_lines(arenas[arm].frame()).len();
                    reports.push((arm, model.drawn(), model.refusal_lines(arenas[arm].frame())));
                    outputs[arm] = Some(draws);
                }
                reports.sort_by_key(|report| report.0);
                assert_eq!(
                    (&reports[0].1, &reports[0].2),
                    (&reports[1].1, &reports[1].2)
                );
                let [direct, installed] = outputs.map(Option::unwrap);
                assert_eq!(direct.draws.len(), installed.draws.len(), "frame {frame}");
                for (a, b) in direct.draws.iter().zip(&installed.draws) {
                    assert_eq!(
                        (a.reference, a.vertices, a.indices, a.first, a.count, a.base),
                        (b.reference, b.vertices, b.indices, b.first, b.count, b.base)
                    );
                    assert_eq!(
                        a.bindings.preparation_proof(),
                        b.bindings.preparation_proof(),
                        "frame {frame}"
                    );
                }
                resources.owners(&models[0], &models[1]);
                resources.view(&globals[0], &globals[1]);
                resources.draws(&models, &direct, &installed);
                assert_eq!(arenas[0].refusals(), arenas[1].refusals());
                assert_eq!(arenas[0].proof_bytes_for_tests(), arenas[1].proof_bytes_for_tests(),
                    "allocator prefix frame {frame}, samples {samples}");
                assert_eq!(objects[0].bytes, objects[1].bytes);
                assert_eq!(objects[0].pixel, objects[1].pixel);
                assert!(
                    objects[0]
                        .retained
                        .stream
                        .same_bindings_mapped_for_tests(&objects[1].retained.stream, |a, b| {
                            resources.view(a, b);
                            true
                        })
                );
                let checkpoints = arenas.each_ref().map(shader_pipeline::gpu::Arena::checkpoint);
                assert_eq!(checkpoints[0].frame(), checkpoints[1].frame());
                let mut other = checkpoints[1].writes();
                for (a, a_offset, a_bytes) in checkpoints[0].writes() {
                    let (b, b_offset, b_bytes) = other.next().expect("matching upload record");
                    resources.buffer(a, b);
                    assert_eq!((a_offset, a_bytes.len(), a_bytes), (b_offset, b_bytes.len(), b_bytes));
                }
                assert!(other.next().is_none());
                drop(other);
                assert_eq!(arenas[0].proof_bytes_for_tests(), arenas[1].proof_bytes_for_tests());
                resources.check();
                for checkpoint in checkpoints {
                    checkpoint.execute(&queue);
                }
                let pixels = render(&direct, kind == Queue::Caster, samples);
                assert_eq!(
                    pixels,
                    render(&installed, kind == Queue::Caster, samples),
                    "frame {frame}, samples {samples}"
                );
                if pixels
                    .chunks_exact(if kind == Queue::Caster { 4 } else { 8 })
                    .any(|pixel| {
                        pixel[..if kind == Queue::Caster { 4 } else { 6 }]
                            .iter()
                            .any(|byte| *byte != 0)
                    })
                {
                    nonempty += 1;
                }
                models[1].recycle_native(installed);
                seen[3] |= !models[1].redirects.is_empty();
                seen[4] |= kind == Queue::Alternate && !direct.draws.is_empty();
            }
            for (model, absent) in models.iter_mut().zip(absent) {
                if let Some(texture) = absent {
                    model.textures[blank] = Some(texture);
                }
                model.material_refusals.clear();
            }
            resources.check();
            for arena in &arenas {
                arena.reset();
            }
        }
    }
    assert!(nonempty > 0 && refusals > 0);
    assert!(seen.into_iter().all(|flag| flag));
    assert!(pollster::block_on(validation.pop()).is_none());
}

#[test]
#[ignore = "requires the pinned shader compiler and a GPU"]
fn native_plan_tail_keeps_independent_late_fatal_prefix_and_recovery() {
    let (_directory, mut library, blank) = fixture();
    library.meshes[1].native[0].lods[0][0].material.shader = Some("Object".into());
    let mut missing = library.meshes[1].clone();
    missing.name = "tail_missing_shared".into();
    missing.native[0].lods[0][0].material.shader = Some("Missing".into());
    let missing_index = library.meshes.len();
    library.meshes.push(missing);
    library.rigs.push(library.rigs[1].clone());
    let (_serial, device, queue) = tests::device();
    let camera = Camera {
        center: [0., 0.], height: 100.,
        map: MapSize { width: 100, height: 100 },
        defines: MapDefines::test(), pose: viewer_gpu::camera::Pose::Map,
    };
    for samples in [1, 4] {
        let mut models = [0, 1].map(|_| {
            let mut model = ModelPass::new(&library, "unit models");
            model.upload_library(&device, &queue, &library).unwrap();
            model
        });
        let arenas = [0, 1].map(|_| shader_pipeline::gpu::Arena::new(&device));
        let mut objects = [ObjectConstants::default(), ObjectConstants::default()];
        let mut resources = independent::Resources::default();
        for frame in 0..10 {
            let fatal = frame == 8;
            let placements = if fatal { vec![1, missing_index] } else { vec![1, 1] };
            for (arm, model) in models.iter_mut().enumerate() {
                model.update_placed(placements.iter().map(|&mesh| (mesh, ModelInstance {
                    scale: 1., ..Default::default()
                })).collect(), 128.);
                if arm == 0 { model.native_walk.installed = false; }
                model.drawn.set(0);
            }
            let mut results = Vec::new();
            for arm in 0..2 {
                let model = &models[arm];
                let globals = &model.textures[blank].as_ref().unwrap().view;
                let frames = [&[frame as u8; 416][..]];
                objects[arm].begin();
                results.push(model.prepare_native(
                    &arenas[arm], ModelFrame { camera, offsets: &[0.], visible: None, seen: None, alpha: 0.75 },
                    Inputs { samples, frames: &frames, grid: &[0; 16], textures: [globals; 10] },
                    Queue::Regular, &mut objects[arm],
                ));
            }
            let [scalar, dense]: [_; 2] = results.try_into().unwrap_or_else(|_| unreachable!());
            if fatal {
                let scalar = scalar.err().expect("the late missing shared program is fatal");
                let dense = dense.err().expect("tail must not turn the fatal into a refusal");
                assert_eq!((scalar.kind(), scalar.to_string()), (dense.kind(), dense.to_string()));
                assert_eq!(scalar.to_string(), "native PdxMeshColor fallback is unavailable");
                assert_eq!(models[0].drawn(), 0, "fatal suppresses the invocation's draw-count commit");
            } else {
                let (scalar, dense) = (scalar.unwrap(), dense.unwrap());
                assert_eq!(scalar.draws.len(), 2);
                assert_eq!(scalar.draws.len(), dense.draws.len());
                for (a, b) in scalar.draws.iter().zip(&dense.draws) {
                    assert_eq!(a.bindings.preparation_proof(), b.bindings.preparation_proof());
                }
                resources.owners(&models[0], &models[1]);
                resources.draws(&models, &scalar, &dense);
            }
            resources.owners(&models[0], &models[1]);
            assert_eq!(models[0].drawn(), models[1].drawn());
            assert_eq!(models[0].refusal_lines(arenas[0].frame()), models[1].refusal_lines(arenas[1].frame()));
            assert_eq!(arenas[0].refusals(), arenas[1].refusals());
            assert_eq!(arenas[0].proof_bytes_for_tests(), arenas[1].proof_bytes_for_tests(), "raw fatal/recovery allocator prefix, frame {frame}, samples {samples}");
            assert_eq!(objects[0].bytes, objects[1].bytes);
            assert_eq!(objects[0].pixel, objects[1].pixel);
            assert!(objects[0].retained.stream.same_bindings_mapped_for_tests(&objects[1].retained.stream, |a, b| {
                resources.view(a, b); true
            }));
            let checkpoints = arenas.each_ref().map(shader_pipeline::gpu::Arena::checkpoint);
            let mut right = checkpoints[1].writes();
            for (a, offset, bytes) in checkpoints[0].writes() {
                let (b, other_offset, other_bytes) = right.next().unwrap();
                resources.buffer(a, b);
                assert_eq!((offset, bytes), (other_offset, other_bytes));
            }
            assert!(right.next().is_none());
            drop(right);
            resources.check();
            for checkpoint in checkpoints { checkpoint.execute(&queue); }
            for arena in &arenas { arena.reset(); }
        }
    }
}

fn same_draws(
    (kept, list): (&DrawList, &kept::Kept),
    fresh: &DrawList,
    residents: &[Resident],
    figures: usize,
) {
    let bytes = list.bytes();
    let same = |a: &RecordedDraw, b: &RecordedDraw| {
        let range = |draw: &RecordedDraw| {
            (draw.reference, draw.vertices, draw.indices, draw.first, draw.count, draw.base)
        };
        range(a) == range(b) && a.bindings.same_resources(&b.bindings)
    };
    assert_eq!(fresh.draws.len(), figures, "the walk draws every figure placed");
    assert_eq!(kept.draws.len(), 1, "one draw a part the walk drew");
    let programs = part_programs(residents);
    let mut covered = vec![false; fresh.draws.len()];
    for (at, keep) in kept.draws.iter().enumerate() {
        let rows = programs(list.key_at(at)).expect("a resident program").program.instanced_rows();
        let (reads, offsets) = keep.bindings.preparation_proof();
        let (block, offsets) = match offsets.split_last() {
            Some((block, offsets)) if !rows.is_empty() => (Some(*block as usize), offsets),
            _ => (None, offsets),
        };
        let base = |binding: u32| {
            let at = block.expect("a rows block") + binding as usize * 4;
            u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes")) as usize
        };
        let walked = list.slots_at(at);
        assert_eq!(keep.instances as usize, walked.len(), "one instance a draw of the walk");
        for (slot, &draw) in walked.iter().enumerate() {
            assert!(!std::mem::replace(&mut covered[draw], true), "each draw of the walk once");
            let fresh = &fresh.draws[draw];
            assert!(same(keep, fresh), "the same pipeline, resources and range");
            let (staged, _) = fresh.bindings.preparation_proof();
            let mut own = offsets.iter();
            for ((binding, staged), (read, _)) in staged.iter().zip(reads) {
                assert_eq!(binding, read);
                let (at, length) = match rows.iter().find(|(row, _)| row == binding) {
                    Some(&(_, stride)) => {
                        let stride = stride as usize;
                        ((base(*binding) + slot) * stride, stride.min(staged.len()))
                    }
                    None => {
                        let at = own.next().expect("a block bound at its own offset");
                        (*at as usize, staged.len())
                    }
                };
                assert_eq!(&bytes[at..at + length], &staged[..length], "binding {binding}, {slot}");
            }
            assert!(own.next().is_none(), "every offset the draw binds is read");
        }
    }
    assert!(covered.into_iter().all(|drawn| drawn), "every draw of the walk");
}

fn kept_fixture() -> (TestDir, ModelLibrary, usize) {
    let (directory, mut library, blank) = fixture();
    let material = &mut library.meshes[1].native[0].lods[0][0].material;
    material.shader = Some("World".into());
    material.shader_file = Some("gfx/FX/kept.shader".into());
    (directory, library, blank)
}

fn placed_kept(x: f32) -> Vec<(usize, ModelInstance)> {
    (0..3)
        .map(|at| {
            (
                1,
                ModelInstance {
                    position: [x + at as f32, 0., 0.],
                    scale: 1.,
                    animation_identity: at as u32 + 1,
                    ..Default::default()
                },
            )
        })
        .collect()
}

#[test]
#[ignore = "requires the pinned shader compiler and a GPU"]
fn retained_casters_equal_a_fresh_walk_and_a_kept_frame_walks_nothing() {
    let (_directory, library, blank) = kept_fixture();
    let (_serial, device, queue) = tests::device();
    let placed = placed_kept;
    let figures = std::cell::Cell::new(3);
    let mut model = ModelPass::new(&library, "map buildings");
    model
        .upload(&device, &queue, &library, ModelField::build(&library, placed(0.), 128.))
        .unwrap();
    model.settle_builds(true);
    let globals = model.textures[blank].as_ref().unwrap().view.clone();
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let mut camera = Camera {
        center: [0., 0.],
        height: 100.,
        map: MapSize {
            width: 100,
            height: 100,
        },
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    };
    let compare = |model: &ModelPass, frame: ModelFrame<'_>, inputs: Inputs<'_>| {
        let fresh = model
            .prepare_native(&arena, frame, inputs, Queue::Caster, &mut ObjectConstants::default())
            .unwrap();
        let renderer = model.native.borrow();
        let native = renderer.as_ref().unwrap();
        let list = &native.casters;
        same_draws((&list.draws, list), &fresh, &native.residents, figures.get());
        let counts = list.counts();
        drop(renderer);
        model.recycle_native(fresh);
        counts
    };
    let step = |model: &mut ModelPass, camera: Camera, scene: u8| {
        arena.reset();
        let scene = [scene; 416];
        let frames = [scene.as_slice()];
        let inputs = Inputs {
            samples: 1,
            frames: &frames,
            grid: &[0; 16],
            textures: [&globals; 10],
        };
        let frame = ModelFrame {
            camera,
            offsets: &[0.],
            visible: None,
            seen: None,
            alpha: 1.,
        };
        let repeated = model.prepare_casters(&arena, frame, inputs).unwrap();
        (compare(model, frame, inputs), repeated)
    };
    let walks = |(counts, _): (kept::Counts, bool)| (counts.walks, counts.kept);
    let repeats = |(_, repeated): (kept::Counts, bool)| repeated;
    assert_eq!(walks(step(&mut model, camera, 1)), (1, 0), "the first frame walks");
    assert_eq!(walks(step(&mut model, camera, 2)), (1, 1), "the same inputs walk nothing");
    assert_eq!(walks(step(&mut model, camera, 3)), (1, 2));
    assert!(!repeats(step(&mut model, camera, 4)), "another shared block");
    assert!(repeats(step(&mut model, camera, 4)), "the same inputs draw the same");
    model.update_placed(placed(0.), 128.);
    assert_eq!(walks(step(&mut model, camera, 4)), (1, 5), "placements made again unchanged");
    assert!(repeats(step(&mut model, camera, 4)), "and draw the same");
    model.update_placed(placed(0.5), 128.);
    let moved = step(&mut model, camera, 4);
    assert!(!repeats(moved), "a move draws otherwise");
    assert_eq!(walks(moved), (1, 7), "a move rewrites its nodes in place");
    assert!(repeats(step(&mut model, camera, 4)));
    camera.center[0] += 1.;
    let travelled = step(&mut model, camera, 4);
    assert_eq!(walks(travelled), (1, 9), "a camera move walks nothing");
    assert!(repeats(travelled));
    assert_eq!(walks(step(&mut model, camera, 7)), (1, 10));
    let mut other = placed(0.5);
    other.remove(0);
    other[0].1.animation_identity = 100;
    model.update_placed(other.clone(), 128.);
    figures.set(2);
    let (counts, repeated) = step(&mut model, camera, 7);
    assert_eq!((counts.walks, counts.spliced, counts.figures), (1, 1, 1));
    assert!(!repeated, "a splice draws otherwise");
    model.update_placed(placed(0.5).into_iter().take(1).chain(other).collect(), 128.);
    figures.set(3);
    let (counts, repeated) = step(&mut model, camera, 7);
    assert_eq!((counts.walks, counts.spliced, counts.figures), (1, 2, 2));
    assert!(!repeated);
    let kept = step(&mut model, camera, 7);
    assert_eq!(walks(kept), (1, 13));
    assert!(repeats(kept));
    model.update_placed(Vec::new(), 128.);
    let nothing = |model: &ModelPass| {
        arena.reset();
        let scene = [7; 416];
        let frames = [scene.as_slice()];
        let inputs = Inputs { samples: 1, frames: &frames, grid: &[0; 16], textures: [&globals; 10] };
        let frame = ModelFrame { camera, offsets: &[0.], visible: None, seen: None, alpha: 1. };
        model.prepare_casters(&arena, frame, inputs).unwrap()
    };
    assert!(!nothing(&model), "casting nothing draws otherwise");
    assert!(nothing(&model), "and nothing again draws the same");
    queue.submit([]);
}

#[test]
#[ignore = "requires the pinned shader compiler and a GPU"]
fn a_kept_list_draws_what_the_frame_draws_and_only_gathers_by_the_candidates() {
    let (_directory, library, blank) = fixture();
    let (_serial, device, queue) = tests::device();
    let placed: Vec<(usize, ModelInstance)> = [10., 70.]
        .into_iter()
        .enumerate()
        .flat_map(|(cluster, x)| {
            (0..library.meshes.len()).map(move |mesh| {
                let identity = (cluster * 100 + mesh + 1) as u32;
                (mesh, ModelInstance { position: [x + mesh as f32, 0., 50.], scale: 1., animation_identity: identity, ..Default::default() })
            })
        })
        .collect();
    let pass = || {
        let mut model = ModelPass::new(&library, "map buildings");
        model.upload(&device, &queue, &library, ModelField::build(&library, placed.clone(), 16.)).unwrap();
        model.settle_builds(true);
        model
    };
    let model = pass();
    let globals = model.textures[blank].as_ref().unwrap().view.clone();
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let camera = Camera {
        center: [40., 50.],
        height: 100.,
        map: MapSize { width: 100, height: 100 },
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    };
    let near = GameCull::of(&camera, [800, 600]);
    let lens = viewer_gpu::camera::Lens { near: 1., far: 20_000., ..viewer_gpu::camera::Lens::MAP };
    let east_eye = Camera {
        center: [40., 50.],
        height: 10.,
        pose: viewer_gpu::camera::Pose::Free(viewer_gpu::camera::Free::new(std::f32::consts::FRAC_PI_2, 0., lens)),
        ..camera
    };
    let step = |model: &ModelPass, camera: Camera, drawn: [f32; 4]| {
        arena.reset();
        let cull = GameCull::of(&camera, [800, 600]);
        model.set_cull(Some(near), Some(cull));
        let scene = [3; 416];
        let frames = [scene.as_slice()];
        let inputs = Inputs { samples: 1, frames: &frames, grid: &[0; 16], textures: [&globals; 10] };
        let frame = ModelFrame { camera, offsets: &[0.], visible: Some([0., 0., 100., 100.]), seen: Some(drawn), alpha: 1. };
        model.prepare_casters(&arena, frame, inputs).unwrap();
        let renderer = model.native.borrow();
        let list = &renderer.as_ref().unwrap().casters;
        let counts = list.counts();
        let mut figures: Vec<usize> = list.units().iter().map(|unit| unit.index).collect();
        figures.sort_unstable();
        figures.dedup();
        (counts.walks, counts.spliced, figures)
    };
    let whole = |camera: Camera, drawn: [f32; 4]| step(&pass(), camera, drawn).2;
    let west = [0., 0., 40., 100.];
    let both = [0., 0., 100., 100.];
    let (walks, _, west_figures) = step(&model, camera, west);
    assert_eq!(walks, 1);
    assert!(!west_figures.is_empty(), "the western figures are drawn");
    assert_eq!(west_figures, whole(camera, west), "the eastern group is gathered and not drawn");
    let (_, _, both_figures) = step(&model, camera, both);
    assert!(both_figures.len() > west_figures.len(), "the ground widened: the eastern figures join");
    assert_eq!(both_figures, whole(camera, both));
    assert_eq!(step(&model, camera, west).2, west_figures, "and leave again");
    let moved = step(&model, east_eye, both).2;
    assert_eq!(moved, whole(east_eye, both), "the cull moved");
    assert!(!moved.is_empty() && moved.len() < both_figures.len(), "the western figures left");
    assert_eq!(step(&model, camera, both).2, both_figures, "and came back");
    queue.submit([]);
}

#[test]
#[ignore = "requires the pinned shader compiler and a GPU"]
fn a_kept_lit_list_equals_a_fresh_lit_walk_and_leaves_the_same_constant_state() {
    let (_directory, library, blank) = kept_fixture();
    let (_serial, device, queue) = tests::device();
    let placed = placed_kept;
    let figures = std::cell::Cell::new(3);
    let mut model = ModelPass::new(&library, "map buildings");
    model
        .upload(&device, &queue, &library, ModelField::build(&library, placed(0.), 128.))
        .unwrap();
    model.settle_builds(true);
    let globals = model.textures[blank].as_ref().unwrap().view.clone();
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let camera = Camera {
        center: [0., 0.],
        height: 100.,
        map: MapSize {
            width: 100,
            height: 100,
        },
        defines: MapDefines::test(),
        pose: viewer_gpu::camera::Pose::Map,
    };
    let step = |model: &mut ModelPass, scene: u8| {
        arena.reset();
        let scene = [scene; 416];
        let frames = [scene.as_slice()];
        let inputs = Inputs {
            samples: 1,
            frames: &frames,
            grid: &[0; 16],
            textures: [&globals; 10],
        };
        let frame = ModelFrame {
            camera,
            offsets: &[0.],
            visible: None,
            seen: None,
            alpha: 1.,
        };
        let (mut kept_state, mut fresh_state) = (ObjectConstants::default(), ObjectConstants::default());
        let kept = model.prepare_kept(&arena, frame, inputs, Queue::Regular, &mut kept_state).unwrap();
        let fresh = model.prepare_native(&arena, frame, inputs, Queue::Regular, &mut fresh_state).unwrap();
        let counts = {
            let renderer = model.native.borrow();
            let native = renderer.as_ref().unwrap();
            let list = &native.lit[0][0];
            same_draws((&kept, list), &fresh, &native.residents, figures.get());
            list.counts()
        };
        assert_eq!(kept_state.bytes, fresh_state.bytes, "the next pass starts from the same block");
        assert_eq!(kept_state.pixel, fresh_state.pixel, "and the same pixel snapshot");
        model.recycle_kept(kept);
        model.recycle_native(fresh);
        counts
    };
    let walks = |counts: kept::Counts| (counts.walks, counts.kept, counts.spliced);
    assert_eq!(walks(step(&mut model, 1)), (1, 0, 0), "the first frame walks");
    assert_eq!(walks(step(&mut model, 2)), (1, 1, 0), "the same inputs walk nothing");
    model.update_placed(placed(0.5), 128.);
    assert_eq!(walks(step(&mut model, 3)), (1, 2, 0), "a move rewrites its nodes in place");
    let mut other = placed(0.5);
    other.remove(0);
    other[0].1.animation_identity = 100;
    model.update_placed(other, 128.);
    figures.set(2);
    assert_eq!(walks(step(&mut model, 4)), (1, 3, 1), "a figure left and another joined");
    assert_eq!(walks(step(&mut model, 5)), (1, 4, 1));
    queue.submit([]);
}

#[test]
#[ignore = "requires a GPU"]
fn a_model_pass_draws_only_after_its_placed_mesh_is_resident() {
    let (_serial, device, queue) = tests::device();
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let pass = ModelPass::with_rigs_for_tests(Vec::new(), "unit models");
    assert!(pass.layers.is_none());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let scene = [0u8; 416];
    let frames = [scene.as_slice()];
    let inputs = Inputs {
        samples: 1,
        frames: &frames,
        grid: &[0; 16],
        textures: [&view; 10],
    };
    let frame = ModelFrame {
        camera: Camera {
            center: [0., 0.],
            height: 2000.,
            map: MapSize {
                width: 100,
                height: 100,
            },
            defines: MapDefines::test(),
            pose: viewer_gpu::camera::Pose::Map,
        },
        offsets: &[0.],
        visible: Some([0., 0., 100., 100.]),
        seen: Some([0., 0., 100., 100.]),
        alpha: 1.,
    };
    let mut object = ObjectConstants::default();
    for queue in [Queue::Regular, Queue::Alternate] {
        let draws = pass
            .prepare_kept(&arena, frame, inputs, queue, &mut object)
            .expect("a pass that draws nothing is no refusal");
        assert!(draws.draws.is_empty());
        pass.recycle_kept(draws);
    }
    pass.prepare_casters(&arena, frame, inputs)
        .expect("nor are its casters");
    assert!(pass.native.borrow().is_none(), "no renderer, so no content stack, was asked for");

    let (_directory, library, _) = fixture();
    let mut pass = ModelPass::new(&library, "unit models");
    pass.settle_builds(true);
    let triangle = library.meshes.iter().position(|mesh| mesh.name == "defined_triangle").unwrap();
    let field = ModelField::build(&library, vec![(triangle, ModelInstance { scale: 1., ..Default::default() })], 128.);
    pass.set_field(&library, field);
    let frame = ModelFrame { visible: None, seen: None, ..frame };
    let error = pass.prepare_kept(&arena, frame, inputs, Queue::Regular, &mut object).err().unwrap();
    assert!(error.to_string().contains("material is not resident"), "{error}");
    assert_eq!(pass.drawn(), 0);
    let error = pass.prepare_casters(&arena, frame, inputs).unwrap_err();
    assert!(error.to_string().contains("material is not resident"), "{error}");

    pass.make_field_resident(&device, &queue).unwrap();
    let first = pass.prepare_kept(&arena, frame, inputs, Queue::Regular, &mut object).unwrap();
    assert_eq!(first.draws.len(), 1, "{:?}", pass.refusal_lines(arena.frame()));
    pass.recycle_kept(first);
}

#[test]
#[ignore = "requires a GPU"]
fn a_model_texture_is_uploaded_once_per_device_and_read_again_once_its_upload_is_gone() {
    let directory = TestDir::new();
    let root = &directory.0;
    std::fs::create_dir_all(root.join("gfx/models")).unwrap();
    let mut tga = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 32, 0x28];
    tga.extend([10u8, 20, 30, 255].repeat(4));
    std::fs::write(root.join("gfx/models/painted.tga"), &tga).unwrap();
    let layers = Arc::new(ResolvedLayers::from_folders(&[(root, &[])]).unwrap());
    let path = "gfx/models/painted.tga".to_owned();
    let decode = Decode::default();
    let mut library = ModelLibrary::empty(decode);
    library
        .textures
        .push(crate::ModelTexture::deferred_for_tests(path, Arc::clone(&layers)));
    library.spaces.push(ColorSpace::Srgb);
    assert!(
        library.textures[0].source().is_none(),
        "a file-backed texture is not decoded on the CPU before upload"
    );
    let (_serial, device, queue) = tests::device();
    let shared = library.device_textures(&device);
    assert!(Arc::ptr_eq(&shared, &library.device_textures(&device)));
    let first = library.gpu_texture(&shared, &device, &queue, "painted", 0).unwrap();
    assert!(
        library.textures[0].source().is_none(),
        "the source bytes stay off the CPU after the device holds the texture"
    );
    let again = library.gpu_texture(&library.device_textures(&device), &device, &queue, "painted", 0).unwrap();
    assert!(Arc::ptr_eq(&first, &again), "a second pass on the device draws the same texture");
    drop((shared, first, again));
    let rebuilt = library.device_textures(&device);
    let reread = library.gpu_texture(&rebuilt, &device, &queue, "painted", 0).unwrap();
    assert_eq!(reread.texture.size().width, 2);
    assert!(library.textures[0].source().is_none());
}

#[test]
fn file_backed_model_textures_stay_deferred_while_in_memory_textures_stay_held() {
    let directory = TestDir::new();
    let root = &directory.0;
    std::fs::create_dir_all(root.join("gfx/models")).unwrap();
    let mut tga = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 32, 0x28];
    tga.extend([10u8, 20, 30, 255].repeat(4));
    std::fs::write(root.join("gfx/models/painted.tga"), &tga).unwrap();
    let layers = Arc::new(ResolvedLayers::from_folders(&[(root, &[])]).unwrap());
    let path = "gfx/models/painted.tga".to_owned();
    let decode = Decode::default();
    let mut library = ModelLibrary::empty(decode);
    library
        .textures
        .push(crate::ModelTexture::deferred_for_tests(path, layers));
    library.spaces.push(ColorSpace::Srgb);
    let blank = library.blank(ColorSpace::Srgb);
    assert!(
        library.textures[0].source().is_none(),
        "a texture backed by the installation stays deferred until upload"
    );
    assert!(
        library.textures[blank].source().is_some(),
        "a texture made in memory has no file to read on upload"
    );
    let pass = ModelPass::new(&library, "warmed");
    assert!(pass.sources[0].0.source().is_none());
    assert!(pass.sources[blank].0.source().is_some());
    assert_eq!(library.textures[0].decode(decode).unwrap().width, 2);
    assert!(library.textures[0].source().is_none());
}
