use super::*;

struct TestDir(std::path::PathBuf);

impl TestDir {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("hoi4_dump_skin_{}_{nonce}", std::process::id()));
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
                map: crate::map::camera::MapSize {
                    width: 100,
                    height: 100,
                },
                defines: crate::map::camera::MapDefines::test(),
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

#[test]
#[ignore = "requires the pinned shader compiler and a GPU"]
fn dump_no_animation_skin_keeps_the_rest_of_the_frame() {
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
            own_radius: 1.,
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
    let (_serial, device, queue) = crate::map::native_shader::tests::device();
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
    let color = texture(crate::map::scene::SCENE_FORMAT);
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
