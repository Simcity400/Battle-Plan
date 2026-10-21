use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use viewer_foundation::map::camera::MapDefines;
use viewer_foundation::map::camera::MapSize;
use viewer_foundation::map::gpu_contract::SCENE_FORMAT;
use viewer_foundation::map::test_support::gpu as tests;

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
                pose: viewer_foundation::map::camera::Pose::Map,
            },
            offsets: &[0., 0.],
            visible: None,
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let directory = TestDir::new();
    let root = &directory.0;
    std::fs::create_dir_all(root.join("gfx/FX")).unwrap();
    // Independently authored geometry and effect, with no installed content.
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

        // A missing shared effect is fatal, never an actor refusal.
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

    // Device capacity errors must also escape material preparation.
    library.textures[blank].width = device.limits().max_texture_dimension_2d + 1;
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

// Independent fixture expectations: only meshes 0/1 can submit. The last two
// descriptors always refuse (except when their opaque queue is not demanded).
fn expected_draw_instances(frame: usize, kind: Queue) -> Vec<usize> {
    let mut result = Vec::new();
    // ObjectShadow/ObjectSkinnedShadow/AlphaShadow read no material textures,
    // and no authored diffuse path admits a caster texture. Removing the
    // material view in frame 4 therefore refuses lit draws only: both views
    // keep three skinned casters plus two parts of each of three rigid actors
    // (2 * (3 + 2 * 3) == 18). A material refusal still precedes every queue.
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
        pose: viewer_foundation::map::camera::Pose::Map,
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
            // The first part staged successfully, but its descriptor's later b3 refusal
            // must leave it cold; this includes queue-filtered and forced-alpha binds.
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

    // Demand a previously unused mesh only after warm frames. Its shared selection error
    // must escape after the identical earlier staging/descriptor prefix, without applying
    // this invocation's Pending state or drawn count.
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
        alpha: 1.,
    };
    let mut errors = Vec::new();
    for (index, object) in objects.iter_mut().enumerate() {
        let arena = &arenas[index];
        model.drawn.set(0);
        let error = model
            .prepare_native(arena, frame, inputs, Queue::Regular, object)
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
