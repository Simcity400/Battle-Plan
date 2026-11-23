use super::*;
use viewer_scene_base::shader_scene::State;
use shader_pipeline::{Buffer, Compiled, ScalarKind, Shader, TextureKind, VertexInput};
#[path = "draw_expected.rs"]
mod expected;

fn packet(frame: usize, actor: usize, shader: &str, count: usize) -> DrawPacket {
    let emitter = super::super::test_support::activation_test_emitter();
    let mut packet = DrawPacket::new(&emitter, 0.75, Transform::IDENTITY, frame as f32 / 60.);
    packet.actor = [1, actor, 0, 0];
    packet.descriptor.shader = shader.into();
    packet.descriptor.billboard = true;
    packet.instances = (0..count)
        .map(|i| Instance {
            position_size: [-0.6 + i as f32 * 0.17, -0.3 + actor as f32 * 0.1, 0.4, 0.18],
            rotation: [i as f32 * 0.07, 0.2, 0.3],
            tile: [i as u8, 2, 3, 4],
            color: [80 + (frame % 150) as u8, 130, 200, 180],
            tail: [0xa5; 12],
        })
        .collect();
    packet
}

#[test]
fn stack_constants_match_expected_native_bytes_bit_for_bit() {
    let values = [
        0.,
        -0.,
        f32::MIN,
        f32::MAX,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::from_bits(0x7fc0_0123),
        f32::from_bits(1),
    ];
    for frame in 0..512 {
        let mut p = packet(frame, 0, "Fixture", 0);
        p.constants.world = std::array::from_fn(|i| {
            std::array::from_fn(|j| values[(frame + i * 4 + j) % values.len()])
        });
        p.constants.scale = [
            values[frame % values.len()],
            values[(frame + 1) % values.len()],
        ];
        p.constants.tiles = [
            i32::MIN.wrapping_add(frame as i32),
            i32::MAX.wrapping_sub(frame as i32),
        ];
        p.constants.time = values[(frame + 2) % values.len()];
        let (scale, object) = expected::constants(&p.constants);
        let actual = DrawConstants::new(&p.constants);
        assert_eq!(scale.as_slice(), bytemuck::bytes_of(&actual.scale));
        assert_eq!(object, actual.object);
    }
}

#[test]
fn binding_reuse_key_compares_bytes_not_float_equality() {
    let a = packet(0, 0, "Fixture", 1);
    let mut b = packet(0, 1, "Fixture", 4);
    assert!(same_packet_bindings(&a, &b));
    let changes: &[fn(&mut DrawPacket)] = &[
        |p| p.page += 1,
        |p| p.constants.world[3][0] = -0.,
        |p| p.constants.scale[1] = -0.,
        |p| p.constants.tiles[0] += 1,
        |p| p.constants.time = -0.,
    ];
    for change in changes {
        let mut b = packet(0, 0, "Fixture", 1);
        change(&mut b);
        assert!(!same_packet_bindings(&a, &b));
    }
    let mut a = a;
    a.constants.time = f32::from_bits(0x7fc0_0123);
    b.constants.time = a.constants.time;
    assert!(same_packet_bindings(&a, &b));
    b.constants.time = f32::from_bits(0x7fc0_0124);
    assert!(!same_packet_bindings(&a, &b));
}

#[test]
fn actor_packing_subranges_equal_each_packet_over_ring_discard_and_append() {
    use shader_pipeline::vertices::{NativeBuffer, pack_native};
    let inputs = inputs(false);
    let layouts = particle_shader::layouts(&inputs, false).unwrap();
    let mut ring = InstanceRing::new(16 * 48);
    for frame in 0..128 {
        let packets: Vec<_> = (0..3)
            .map(|i| packet(frame, 0, "Fixture", 1 + (frame + i) % 4))
            .collect();
        let start = ring
            .append(packets.iter().flat_map(|p| &p.instances))
            .unwrap();
        let allocation = ring.allocation.lock().unwrap();
        let pack = |offset, count| {
            pack_native(
                &layouts[1..],
                &BTreeMap::from([(
                    0,
                    Some(NativeBuffer {
                        bytes: &allocation.bytes,
                        size: allocation.size,
                        offset,
                        known: &allocation.known,
                    }),
                )]),
                4,
                count,
            )
            .unwrap()
        };
        let full = pack(
            start,
            packets.iter().map(|p| p.instances.len() as u32).sum(),
        );
        let stride = full.layouts[0].stride as usize;
        let mut first = 0;
        for p in &packets {
            let count = p.instances.len();
            let old = pack(start + (first * 48) as u64, count as u32);
            assert_eq!(old.layouts, full.layouts);
            assert_eq!(
                old.bytes[0],
                full.bytes[0][first * stride..(first + count) * stride]
            );
            first += count;
        }
    }
}

#[test]
fn weather_noop_binds_match_bytes_ranges_and_handles_over_many_frames() {
    let quad: Arc<[u8]> = bytemuck::cast_slice(&particle_shader::QUAD).into();
    let values = [
        0.,
        -0.,
        f32::from_bits(0x7fc0_0123),
        f32::from_bits(0x7fc0_0124),
        1.,
        -2.,
    ];
    let mut old = State::default();
    let mut new = State::default();
    let mut skipped = 0;
    for frame in 0..512 {
        let mut applied = Applied::default();
        let light = [frame as u8; 16];
        for state in [&mut old, &mut new] {
            state.stream.bind_constants(2, Stage::Pixel, Some(&light));
            state
                .stream
                .bind_partial_constants(2, Stage::Vertex, Some(&light), &[0..4, 8..16]);
        }
        for actor in 0..8 {
            let mut p = packet(frame, actor, "Fixture", 0);
            p.constants.scale = [
                values[(frame + actor) % values.len()],
                values[actor % values.len()],
            ];
            p.constants.time = values[(frame + actor * 3) % values.len()];
            p.constants.world[3][0] = values[(frame + actor * 5) % values.len()];
            let constants = DrawConstants::new(&p.constants);
            for repeat in 0..3 {
                let scale = bytemuck::bytes_of(&constants.scale);
                for stage in [Stage::Vertex, Stage::Pixel] {
                    old.stream.bind_partial_constants(
                        2,
                        stage,
                        Some(scale),
                        std::slice::from_ref(&(0..8)),
                    );
                }
                skipped += usize::from(applied.scale(&mut new, scale));
                assert!(
                    old.same_bindings(&new),
                    "scale frame {frame}, actor {actor}, repeat {repeat}"
                );
                old.bind_vertex(0, 8, quad.clone());
                skipped += usize::from(applied.quad(&mut new, &quad));
                assert!(old.same_bindings(&new), "quad frame {frame}, actor {actor}");
                if actor % 3 == 1 {
                    continue;
                }
                old.stream.bind_partial_constants(
                    3,
                    Stage::Vertex,
                    Some(&constants.object),
                    std::slice::from_ref(&(0..84)),
                );
                skipped += usize::from(applied.object(&mut new, &constants.object));
                assert!(
                    old.same_bindings(&new),
                    "object frame {frame}, actor {actor}"
                );
                if repeat == 1 {
                    let allocation = allocation(56 * 28);
                    for state in [&mut old, &mut new] {
                        state.bind_partial_vertex(0, 28, allocation.clone(), 0);
                    }
                    applied.quad = false;
                    {
                        let mut bytes = allocation.lock().unwrap();
                        bytes.bytes = vec![0; 28];
                        bytes.known = std::iter::once(0..20).collect();
                    }
                    assert!(old.same_bindings(&new));
                }
            }
        }
        assert!(old.snapshot().same_bindings(&new.snapshot()));
    }
    assert!(
        skipped > 10_000,
        "bitwise-equal writes really took the no-op route"
    );
}

#[test]
fn weather_technique_names_are_cached_by_authored_flags_without_aliasing_programs() {
    content_reader_script::test_game::enter();
    let effect = effect(false);
    let names = [
        "Fixture",
        "Alternate",
        "MissingTrail",
        "Missing",
        "MissingNoBillboard",
        "ModParticle-雪",
    ];
    let mut cached = Techniques::default();
    let mut expected = BTreeMap::new();
    for frame in 0..128 {
        for name in names {
            for trail in [false, true] {
                for billboard in [false, true] {
                    let mut p = packet(frame, 0, name, 1);
                    p.descriptor.trail = trail;
                    p.descriptor.billboard = billboard;
                    let key = (p.descriptor.technique(), trail);
                    let index = cached.get(&effect, &p.descriptor).unwrap();
                    let flags = usize::from(trail) * 2 + usize::from(billboard);
                    assert_eq!(cached.names[name][flags], Some(index));
                    assert_eq!(cached.programs[index].name, key.0);
                    assert_eq!(cached.programs[index].trail, trail);
                    let original = expected.entry(key.clone()).or_insert_with(|| index);
                    assert_eq!(
                        *original, index,
                        "aliases of one native name/trail share selection"
                    );
                    let selection = effect.select(&key.0).unwrap();
                    let actual = cached.programs[index]
                        .program
                        .as_ref()
                        .map(|p| &p.selection);
                    match (selection.as_ref(), actual) {
                        (Some(expected), Some(actual)) => {
                            assert!(Arc::ptr_eq(&expected.effect, &actual.effect));
                            assert_eq!(expected.index, actual.index);
                        }
                        (None, None) => {}
                        _ => panic!("cached selection differs for {key:?}"),
                    }
                }
            }
        }
    }
    assert_eq!(cached.names.len(), names.len());
    assert!(
        cached
            .names
            .values()
            .all(|flags| flags.iter().all(Option::is_some))
    );
    assert_eq!(cached.programs.len(), expected.len());
}

fn inputs(trail: bool) -> Vec<VertexInput> {
    let specs = if trail {
        vec![
            ("POSITION0", ScalarKind::Float, 3),
            ("TEXCOORD0", ScalarKind::Float, 2),
            ("TEXCOORD1", ScalarKind::Uint, 4),
            ("COLOR0", ScalarKind::Float, 4),
        ]
    } else {
        vec![
            ("TEXCOORD0", ScalarKind::Float, 2),
            ("TEXCOORD1", ScalarKind::Float, 4),
            ("TEXCOORD2", ScalarKind::Float, 3),
            ("TEXCOORD3", ScalarKind::Uint, 4),
            ("COLOR0", ScalarKind::Float, 4),
        ]
    };
    specs
        .into_iter()
        .enumerate()
        .map(|(location, (semantic, kind, components))| VertexInput {
            location: location as u32,
            semantic: semantic.into(),
            kind,
            components,
        })
        .collect()
}

fn effect(replacement: bool) -> Arc<Effect> {
    let source = Arc::new(content_reader_gui::pdxshader::program::read_with(b"weather-proof.shader", &|path| {
        Some(if path == b"weather-proof.shader" {
            b"RasterizerState Visible { CullMode = none } Effect Fixture { RasterizerState = Visible } Effect Alternate { RasterizerState = Visible } Effect FixtureTrail { RasterizerState = Visible } Effect FixtureTrailNoBillboard { RasterizerState = Visible } Effect UnknownScale { RasterizerState = Visible }".to_vec()
        } else { Vec::new() })
    }).unwrap());
    let shader = |wgsl: &str| Shader {
        wgsl: wgsl.into(),
        independent_draws: false,
        buffers: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: Vec::new(),
        clock: Default::default(),
    };
    let common = r#"
        struct Object { world: mat4x4<f32>, atlas: vec4<f32>, time: vec4<f32> }
        @group(0) @binding(4) var<uniform> scale: vec4<f32>;
        @group(0) @binding(6) var<uniform> object: Object;
        struct Out { @builtin(position) position: vec4<f32>, @location(0) colour: vec4<f32> }
    "#;
    let ordinary = r#"
        @vertex fn main(@location(0) uv: vec2<f32>, @location(1) pos: vec4<f32>,
            @location(2) rotation: vec3<f32>, @location(3) tile: vec4<u32>, @location(4) colour: vec4<f32>) -> Out {
            var out: Out;
            out.position = object.world * vec4<f32>((uv*2.-1.)*pos.w*scale.x + pos.xy, pos.z, 1.);
            out.colour = colour * vec4<f32>(1.-rotation.x*0.1, 1.-f32(tile.x)*0.01,
                0.8+object.time.x*0.01+object.atlas.z*0.01, scale.y);
            return out;
        }
    "#;
    let trail = r#"
        @vertex fn main(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>,
            @location(2) tile: vec4<u32>, @location(3) colour: vec4<f32>) -> Out {
            var out: Out; out.position = object.world * vec4<f32>(pos,1.);
            out.colour = colour * vec4<f32>(1.-uv.x*0.01,1.-f32(tile.x)*0.01,
                0.8+object.time.x*0.01+object.atlas.z*0.01,scale.y);
            return out;
        }
    "#;
    let mut pixel = shader(
        r#"
        @group(1) @binding(1) var image: texture_2d<f32>;
        @fragment fn main(@location(0) colour: vec4<f32>) -> @location(0) vec4<f32> {
            return colour * textureLoad(image, vec2<i32>(0), 0);
        }
    "#,
    );
    pixel.textures.push((1, TextureKind::D2));
    let compiled: Vec<_> = (0..5)
        .map(|index| {
            let is_trail = index == 2 || index == 3;
            let mut vertex = shader(&format!(
                "{common}{}",
                if is_trail { trail } else { ordinary }
            ));
            vertex.inputs = inputs(is_trail);
            if index == 1 {
                vertex.inputs.reverse();
            }
            vertex.buffers = vec![
                Buffer {
                    binding: 4,
                    size: 16,
                    reads: std::iter::once(if index == 4 { 0..12 } else { 0..8 }).collect(),
                },
                Buffer {
                    binding: 6,
                    size: 96,
                    reads: std::iter::once(0..84).collect(),
                },
            ];
            Ok(Arc::new(Compiled {
                vertex,
                pixel: pixel.clone(),
            }))
        })
        .collect();
    let recorded = Effect::from_compiled(source.clone(), Vec::new(), compiled.clone());
    if replacement {
        let mut broken = compiled;
        let mut alternate = broken[1].as_ref().unwrap().as_ref().clone();
        alternate.vertex.wgsl = alternate.vertex.wgsl.replace("fn main(", "fn absent(");
        broken[1] = Ok(Arc::new(alternate));
        Effect::replacing(source, broken, recorded)
    } else {
        recorded
    }
}

fn frame_packets(frame: usize) -> Vec<DrawPacket> {
    let mut packets = Vec::new();
    for actor in 0..4 {
        for (ordinal, shader) in ["Fixture", "Fixture", "Alternate", "Alternate", "Fixture"]
            .into_iter()
            .enumerate()
        {
            let mut p = packet(frame, actor, shader, 1 + (frame + ordinal) % 2);
            p.page = usize::from(ordinal == 4);
            if ordinal == 4 {
                p.constants.world[3][0] = 0.05;
            }
            packets.push(p);
        }
        packets.push(packet(frame, actor, "Missing", 0));
        let mut p = packet(frame, actor, "Fixture", frame % 4);
        p.descriptor.trail = true;
        p.descriptor.billboard = frame.is_multiple_of(2);
        packets.push(p);
    }
    let mut empty = packet(frame, 4, "Missing", 0);
    empty.constants.scale = [
        if frame.is_multiple_of(2) {
            -0.
        } else {
            f32::from_bits(0x7fc0_0123)
        },
        0.5,
    ];
    packets.push(empty);
    let mut p = packet(frame, 5, "Missing", 3);
    if frame.is_multiple_of(2) {
        p.descriptor.trail = true;
    }
    packets.push(p);
    if frame % 9 == 2 {
        packets.push(packet(frame, 6, "UnknownScale", 2));
    }
    order(&mut packets);
    packets
}

fn delta(before: &BTreeMap<String, u64>, after: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    after
        .iter()
        .filter_map(|(key, value)| {
            let count = value - before.get(key).copied().unwrap_or(0);
            if count != 0 {
                Some((key.clone(), count))
            } else {
                None
            }
        })
        .collect()
}

#[test]
#[ignore = "requires a GPU"]
fn weather_draws_match_fixed_bytes_states_and_pixels_many_frames() {
    content_reader_script::test_game::enter();
    let (_serial, device, queue) = viewer_gpu::test_support::gpu::device();
    let image = |rgba: [u8; 4]| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle proof page"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            texture.size(),
        );
        texture.create_view(&Default::default())
    };
    let pages = vec![image([255_u8, 210, 170, 255]), image([160, 255, 210, 255])];
    let targets = Targets::new(&device, [16, 16], [16, 16], 1, viewer_gpu::gpu_contract::SeaBed::Half);
    let arena = gpu::Arena::new(&device);
    let expected_arena = gpu::Arena::new(&device);
    let frame_bytes = [0_u8; 416];
    let grid = [0_u8; 16];
    let projection = matrix(Transform::IDENTITY);
    let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fixture expected indices"),
        contents: bytemuck::cast_slice(&[0_u16, 1, 2, 2, 1, 3]),
        usage: wgpu::BufferUsages::INDEX,
    });
    let template = |trail| gpu::Pass {
        vertices: Vec::new(),
        primitive: wgpu::PrimitiveState {
            topology: if trail {
                wgpu::PrimitiveTopology::TriangleStrip
            } else {
                wgpu::PrimitiveTopology::TriangleList
            },
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: gpu_contract::DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        targets: vec![Some(wgpu::ColorTargetState {
            format: gpu_contract::SCENE_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })],
        instanced: 0,
        stream: None,
        vertex_color_tint: None,
    };
    let clear = |encoder: &mut wgpu::CommandEncoder| {
        let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particle proof clear"),
            color_attachments: &[Some(
                targets.scene_attachment(wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)),
            )],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Clear(1.))),
            ..Default::default()
        });
    };
    let mut prepared = 0;
    let mut planned = 0;
    let mut reused = 0;
    let mut refused = 0;
    let mut visible = 0;
    for replacement in [false, true] {
        let effect = effect(replacement);
        let mut renderer = Renderer::new(&device, effect.clone(), pages.clone(), 1, 16);
        let mut retained = State::default();
        let mut expected_retained = State::default();
        let mut expected_consumer = State::default();
        for frame in 0..48 {
            let packets = if frame % 13 == 12 {
                Vec::new()
            } else {
                frame_packets(frame)
            };
            renderer.pages.rotate_left(usize::from(frame % 7 == 3));
            let expected = expected::frame(
                &effect,
                &renderer.pages,
                Inputs {
                    frame: &frame_bytes,
                    grid: &grid,
                    textures: std::array::from_fn(|_| &pages[frame % 2]),
                    retained: &mut expected_retained,
                },
                &packets,
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            clear(&mut encoder);
            renderer.trace = Some(Vec::new());
            let before_errors = renderer.retained.stream.refused.clone();
            let before_cache_errors = renderer.retained.stream.cache.refused.clone();
            let result = renderer
                .draw(
                    &arena,
                    &mut encoder,
                    &targets,
                    projection,
                    [0., 0., 2.],
                    0,
                    Inputs {
                        frame: &frame_bytes,
                        grid: &grid,
                        textures: std::array::from_fn(|_| &pages[frame % 2]),
                        retained: &mut retained,
                    },
                    &packets,
                )
                .err()
                .map(|error| error.to_string());
            let errors = delta(&before_errors, &renderer.retained.stream.refused);
            let cache_errors = delta(
                &before_cache_errors,
                &renderer.retained.stream.cache.refused,
            );
            let trace = renderer.trace.take().unwrap();
            arena.flush(&queue);
            queue.submit([encoder.finish()]);
            let pixels =
                gpu_contract::read_texture(&device, &queue, targets.scene_texture(), 8).unwrap();

            let mut encoder = device.create_command_encoder(&Default::default());
            clear(&mut encoder);
            let before_errors = expected_consumer.stream.refused.clone();
            let before_cache_errors = expected_consumer.stream.cache.refused.clone();
            let mut expected_result = None;
            let mut completed = 0;
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("fixed particle fixture states"),
                    color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
                    depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
                    ..Default::default()
                });
                pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint16);
                for draw in &expected.draws {
                    expected_consumer.inherit(&draw.state);
                    let submission = match expected_consumer.prepare(
                        &expected_arena,
                        format_args!("particle {}", draw.name),
                        &template(draw.trail),
                        draw.vertices,
                        draw.instances,
                    ) {
                        Ok(submission) => submission,
                        Err(error) => {
                            expected_result = Some(error.to_string());
                            break;
                        }
                    };
                    let actual = &trace[completed];
                    assert_eq!(actual.packet, draw.packet, "source order, frame {frame}");
                    let packed = draw
                        .state
                        .packed_bytes(draw.vertices, draw.instances)
                        .unwrap();
                    assert_eq!(
                        actual.layouts, packed.layouts,
                        "layouts, frame {frame}, packet {}",
                        draw.packet
                    );
                    assert_eq!(
                        actual.bytes, packed.bytes,
                        "vertices, frame {frame}, packet {}",
                        draw.packet
                    );
                    let (scale, object) = expected::constants(&packets[draw.packet].constants);
                    let constants = vec![(4, scale.to_vec()), (6, object.to_vec())];
                    assert_eq!(
                        actual.bindings.preparation_proof().0,
                        constants,
                        "constants, frame {frame}, packet {}",
                        draw.packet
                    );
                    assert_eq!(
                        actual.bindings.preparation_proof().0,
                        submission.pass.bindings.preparation_proof().0
                    );
                    assert!(
                        actual.retained.same_bindings(&draw.state),
                        "retained bytes/ranges/resources, frame {frame}, packet {}",
                        draw.packet
                    );
                    prepared += usize::from(actual.prepared);
                    planned += usize::from(actual.planned);
                    reused += usize::from(actual.reused);
                    pass.set_pipeline(&submission.pass.pipeline.pipeline);
                    pass.set_stencil_reference(submission.pass.stencil_reference.unwrap_or(0));
                    pass.set_blend_constant(wgpu::Color::WHITE);
                    submission.pass.bindings.bind(&mut pass);
                    for (slot, vertices) in submission.vertices.iter().enumerate() {
                        pass.set_vertex_buffer(slot as u32, vertices.slice());
                    }
                    if draw.trail {
                        pass.draw(0..draw.vertices, 0..draw.instances);
                    } else {
                        pass.draw_indexed(0..6, 0, 0..draw.instances);
                    }
                    completed += 1;
                }
            }
            assert_eq!(trace.len(), completed, "draw count, frame {frame}");
            assert_eq!(
                result, expected_result,
                "refusal, frame {frame}, replacement {replacement}"
            );
            assert_eq!(
                errors,
                delta(&before_errors, &expected_consumer.stream.refused)
            );
            assert_eq!(
                cache_errors,
                delta(
                    &before_cache_errors,
                    &expected_consumer.stream.cache.refused
                )
            );
            refused += errors.values().sum::<u64>();
            if expected_result.is_none() {
                expected_retained.inherit(&expected.final_state);
                assert!(renderer.retained.same_bindings(&expected.final_state));
            } else {
                assert!(renderer.retained.same_bindings(&expected_consumer));
            }
            assert!(retained.same_bindings(&expected_retained));
            expected_arena.flush(&queue);
            queue.submit([encoder.finish()]);
            let expected_pixels =
                gpu_contract::read_texture(&device, &queue, targets.scene_texture(), 8).unwrap();
            assert_eq!(
                pixels, expected_pixels,
                "pixels, frame {frame}, replacement {replacement}"
            );
            visible += pixels.iter().filter(|&&byte| byte != 0).count();
            arena.reset();
            expected_arena.reset();
        }
    }
    assert!(prepared > 500, "per-actor uploads exercised");
    assert!(planned > 500, "layout-derived copy plans exercised");
    assert!(reused > 500, "within-view bindings reused");
    assert!(refused > 0, "unknown constant reads refused");
    assert!(visible > 0, "pixel proof includes visible geometry");
}

#[test]
#[ignore = "requires a GPU"]
fn weather_native_cache_preserves_keys_ranges_resources_and_late_upload_failures() {
    content_reader_script::test_game::enter();
    let _serial = viewer_gpu::test_support::serial();
    let instance =
        shader_pipeline::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let mut descriptor = gpu_contract::device_descriptor(&adapter);
    descriptor.required_limits.max_buffer_size = 4 << 20;
    let (device, _queue) = pollster::block_on(adapter.request_device(&descriptor)).unwrap();
    let image = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("native cache resource proof"),
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
    let views = [
        image.create_view(&Default::default()),
        image.create_view(&Default::default()),
    ];
    let effect = effect(false);
    let program = Program::new(&effect, "Fixture", false).unwrap().unwrap();
    packing_upload_refusal_and_retry(&device, &program);
    let mut state = State::default();
    state
        .stream
        .select_program(&program.selection, &program.layouts);
    state
        .stream
        .bind_blend(program.state.blend, program.state.write_mask);
    state.stream.bind_rasterizer(program.raster.clone());
    state
        .stream
        .bind_texture(0, Stage::Pixel, Some(views[0].clone()), None);
    let constants = DrawConstants::new(&packet(0, 0, "Fixture", 1).constants);
    state.stream.bind_partial_constants(
        2,
        Stage::Vertex,
        Some(bytemuck::bytes_of(&constants.scale)),
        std::slice::from_ref(&(0..8)),
    );
    state.stream.bind_partial_constants(
        3,
        Stage::Vertex,
        Some(&constants.object),
        std::slice::from_ref(&(0..84)),
    );
    state.bind_vertex(0, 8, bytemuck::cast_slice(&particle_shader::QUAD).into());
    let template = gpu::Pass {
        vertices: Vec::new(),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: Default::default(),
        targets: vec![Some(wgpu::ColorTargetState {
            format: gpu_contract::SCENE_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })],
        instanced: 0,
        stream: None,
        vertex_color_tint: None,
    };
    let old_arena = gpu::Arena::new(&device);
    let arena = gpu::Arena::new(&device);
    let mut cache = shader_pipeline::stream::NativePassCache::default();
    let snapshots = |cache: &shader_pipeline::stream::NativePassCache| {
        cache.binding_textures().cloned().collect::<Vec<_>>()
    };
    type Snapshot = Arc<BTreeMap<u32, Option<gpu::TextureBinding>>>;
    let same_snapshots = |a: &[Snapshot], b: &[Snapshot]| {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b))
    };
    let count = (device.limits().max_buffer_size / 36 + 1) as u32;
    let big: Arc<[u8]> = vec![0; count as usize * 48].into();
    let small: Arc<[u8]> = vec![0; 48].into();
    for step in 0..4 {
        let too_large = step < 2;
        state.bind_vertex(
            1,
            48,
            if too_large {
                big.clone()
            } else {
                small.clone()
            },
        );
        let count = if too_large { count } else { 1 };
        let before = state.stream.refused.clone();
        let old = state.prepare(&old_arena, "late vertex failure", &template, 4, count);
        let old_errors = delta(&before, &state.stream.refused);
        let before = state.stream.refused.clone();
        let new = state.prepare_with_cache(
            &arena,
            "late vertex failure",
            &template,
            4,
            count,
            None,
            None,
            Some(&mut cache),
        );
        assert_eq!(old_errors, delta(&before, &state.stream.refused));
        match (old, new) {
            (Err(a), Err(b)) => {
                assert!(too_large);
                assert_eq!(a.to_string(), b.to_string());
                assert!(a.to_string().contains("buffer limit"));
            }
            (Ok(a), Ok(b)) => {
                assert!(!too_large);
                assert!(a.pass.bindings.same_resources(&b.pass.bindings));
                assert_eq!(
                    a.pass.bindings.preparation_proof().0,
                    b.pass.bindings.preparation_proof().0
                );
            }
            _ => panic!("native upload error parity, step {step}"),
        }
        assert_eq!(
            cache.binding_textures().count(),
            usize::from(step >= 2),
            "refused draws never publish a new program"
        );
        old_arena.reset();
        arena.reset();
    }
    let warm = snapshots(&cache);
    for known in [0..8, 0..4, 0..8] {
        state.stream.bind_partial_constants(
            2,
            Stage::Vertex,
            Some(bytemuck::bytes_of(&constants.scale)),
            std::slice::from_ref(&known),
        );
        let old = state.prepare(&old_arena, "warm range proof", &template, 4, 1);
        let new = state.prepare_with_cache(
            &arena,
            "warm range proof",
            &template,
            4,
            1,
            None,
            None,
            Some(&mut cache),
        );
        match (old, new) {
            (Ok(a), Ok(b)) => {
                assert_eq!(known.end, 8);
                assert_eq!(
                    a.pass.bindings.preparation_proof().0,
                    b.pass.bindings.preparation_proof().0
                );
            }
            (Err(a), Err(b)) => {
                assert_eq!(known.end, 4);
                assert_eq!(a.to_string(), b.to_string());
            }
            _ => panic!("warm cache skipped a changed initialized range"),
        }
        old_arena.reset();
        arena.reset();
    }
    assert_eq!(cache.binding_textures().count(), 1);
    assert_eq!(
        *warm[0],
        **cache.binding_textures().next().unwrap(),
        "range recovery keeps the same complete resource identity"
    );

    fn compare_success(
        state: &mut State,
        old: &gpu::Arena,
        new: &gpu::Arena,
        template: &gpu::Pass,
        cache: &mut shader_pipeline::stream::NativePassCache,
    ) {
        let a = state
            .prepare(old, "alternating resource proof", template, 4, 1)
            .unwrap();
        let b = state
            .prepare_with_cache(
                new,
                "alternating resource proof",
                template,
                4,
                1,
                None,
                None,
                Some(cache),
            )
            .unwrap();
        assert!(a.pass.bindings.same_resources(&b.pass.bindings));
        assert_eq!(
            a.pass.bindings.preparation_proof().0,
            b.pass.bindings.preparation_proof().0
        );
        assert_eq!(a.pass.stencil_reference, b.pass.stencil_reference);
        assert_eq!(a.pass.scissor_enabled, b.pass.scissor_enabled);
    }
    for frame in 0..32 {
        for draw in 0..4 {
            let mut constants =
                DrawConstants::new(&packet(frame * 4 + draw, 0, "Fixture", 1).constants);
            constants.object[80..84].copy_from_slice(&((frame * 4 + draw) as f32).to_le_bytes());
            state.stream.bind_partial_constants(
                3,
                Stage::Vertex,
                Some(&constants.object),
                std::slice::from_ref(&(0..84)),
            );
            state
                .stream
                .bind_texture(0, Stage::Pixel, Some(views[draw % 2].clone()), None);
            compare_success(&mut state, &old_arena, &arena, &template, &mut cache);
        }
        old_arena.reset();
        arena.reset();
    }
    let warm = snapshots(&cache);
    assert_eq!(
        warm.len(),
        2,
        "A/B/A returns do not accumulate new captures"
    );
    assert_eq!(warm[0][&binding(0, Stage::Pixel)], Some(views[0].clone().into()));
    assert_eq!(warm[1][&binding(0, Stage::Pixel)], Some(views[1].clone().into()));
    let third = image.create_view(&Default::default());
    state
        .stream
        .bind_texture(0, Stage::Pixel, Some(third.clone()), None);
    state.bind_vertex(1, 48, big.clone());
    for _ in 0..2 {
        let errors = state.stream.refused.clone();
        let a = state
            .prepare(&old_arena, "warm late vertex failure", &template, 4, count)
            .err()
            .unwrap();
        let old_errors = delta(&errors, &state.stream.refused);
        let errors = state.stream.refused.clone();
        let b = state
            .prepare_with_cache(
                &arena,
                "warm late vertex failure",
                &template,
                4,
                count,
                None,
                None,
                Some(&mut cache),
            )
            .err()
            .unwrap();
        assert_eq!(a.to_string(), b.to_string());
        assert!(a.to_string().contains("buffer limit"));
        assert_eq!(old_errors, delta(&errors, &state.stream.refused));
        assert!(
            same_snapshots(&warm, &snapshots(&cache)),
            "failed third page is not published and warm A/B survive"
        );
    }
    state.bind_vertex(1, 48, small.clone());
    for view in [&views[0], &third, &views[1], &third] {
        state
            .stream
            .bind_texture(0, Stage::Pixel, Some(view.clone()), None);
        compare_success(&mut state, &old_arena, &arena, &template, &mut cache);
    }
    let recovered = snapshots(&cache);
    assert_eq!(
        recovered.len(),
        3,
        "third page publishes once, after successful geometry"
    );
    assert!(same_snapshots(&warm, &recovered[..2]));
    assert_eq!(recovered[2][&binding(0, Stage::Pixel)], Some(third.clone().into()));
    for _ in 0..2 {
        old_arena.reset();
        arena.reset();
    }
    for (index, view) in [&views[0], &views[1], &third].into_iter().enumerate() {
        state
            .stream
            .bind_texture(0, Stage::Pixel, Some(view.clone()), None);
        compare_success(&mut state, &old_arena, &arena, &template, &mut cache);
        assert_eq!(
            cache.binding_textures().count(),
            index + 1,
            "a frame gap prunes all earlier entries before rewarming"
        );
    }
    assert_eq!(cache.binding_textures().count(), 3);
    old_arena.reset();
    arena.reset();
    state
        .stream
        .bind_texture(0, Stage::Pixel, Some(views[0].clone()), None);
    for frame in 0..16 {
        for draw in 0..4 {
            match draw {
                0 => state.stream.forget_texture(5, Stage::Pixel, None),
                1 | 2 => {
                    state
                        .stream
                        .bind_texture(5, Stage::Pixel, Some(views[draw - 1].clone()), None)
                }
                _ => state.stream.bind_texture(5, Stage::Pixel, None, None),
            }
            compare_success(&mut state, &old_arena, &arena, &template, &mut cache);
            let held = snapshots(&cache);
            assert_eq!(
                held.len(),
                if frame == 0 { 3 + draw } else { 4 },
                "complete map gate, frame {frame}, draw {draw}"
            );
            let unread = binding(5, Stage::Pixel);
            let expected = match draw {
                0 => None,
                1 | 2 => Some(Some(views[draw - 1].clone().into())),
                _ => Some(None),
            };
            assert!(
                held.iter().any(|map| {
                    map[&binding(0, Stage::Pixel)] == Some(views[0].clone().into())
                        && map.get(&unread).cloned() == expected
                }),
                "complete map includes unread missing/view/null identity"
            );
        }
        old_arena.reset();
        arena.reset();
    }
    assert_eq!(cache.binding_textures().count(), 4);
    for frame in 0..128 {
        let mut template = template.clone();
        template.primitive.topology = if frame % 3 == 0 {
            wgpu::PrimitiveTopology::TriangleStrip
        } else {
            wgpu::PrimitiveTopology::TriangleList
        };
        template.targets[0].as_mut().unwrap().format = if frame % 5 == 0 {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            gpu_contract::SCENE_FORMAT
        };
        let mut raster = program.raster.clone();
        raster.scissor = frame % 2 == 0;
        state.stream.bind_rasterizer(raster);
        state.stream.bind_blend(
            None,
            if frame % 7 == 0 {
                wgpu::ColorWrites::RED
            } else {
                wgpu::ColorWrites::ALL
            },
        );
        state
            .stream
            .bind_vertex_stride(0, if frame % 11 == 0 { 4 } else { 8 });
        let mut constants = DrawConstants::new(&packet(frame, 0, "Fixture", 1).constants);
        constants.object[95] = frame as u8;
        state.stream.bind_partial_constants(
            2,
            Stage::Vertex,
            Some(bytemuck::bytes_of(&constants.scale)),
            std::slice::from_ref(&(0..if frame % 13 == 0 { 4 } else { 8 })),
        );
        state.stream.bind_partial_constants(
            3,
            Stage::Vertex,
            Some(&constants.object),
            std::slice::from_ref(&(0..84)),
        );
        match frame % 9 {
            0 => state.stream.forget_texture(0, Stage::Pixel, None),
            1 => state.stream.bind_texture(0, Stage::Pixel, None, None),
            _ => state
                .stream
                .bind_texture(0, Stage::Pixel, Some(views[frame % 2].clone()), None),
        }
        let before = state.stream.refused.clone();
        let old = state.prepare(&old_arena, "ordered cache proof", &template, 4, 1);
        let old_errors = delta(&before, &state.stream.refused);
        let before = state.stream.refused.clone();
        let new = state.prepare_with_cache(
            &arena,
            "ordered cache proof",
            &template,
            4,
            1,
            None,
            None,
            Some(&mut cache),
        );
        assert_eq!(old_errors, delta(&before, &state.stream.refused));
        match (old, new) {
            (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string()),
            (Ok(a), Ok(b)) => {
                assert!(a.pass.bindings.same_resources(&b.pass.bindings));
                assert_eq!(
                    a.pass.bindings.preparation_proof().0,
                    b.pass.bindings.preparation_proof().0
                );
                assert_eq!(a.pass.stencil_reference, b.pass.stencil_reference);
                assert_eq!(a.pass.scissor_enabled, b.pass.scissor_enabled);
            }
            _ => panic!("ordered cache proof outcome mismatch, frame {frame}"),
        }
        old_arena.reset();
        arena.reset();
    }
}

#[test]
fn weather_ring_append_matches_expected_generations_ranges_bytes_and_errors() {
    let mut ring = InstanceRing::new(16 * 48);
    let mut cursor = 0;
    let mut bytes = Vec::new();
    let mut known = Vec::new();
    for frame in 0..512 {
        let packets: Vec<_> = (0..3)
            .map(|actor| {
                packet(
                    frame,
                    actor,
                    "Fixture",
                    if frame % 17 == 0 {
                        6
                    } else {
                        (frame + actor) % 5
                    },
                )
            })
            .collect();
        let count: usize = packets.iter().map(|packet| packet.instances.len()).sum();
        let previous = ring.allocation.clone();
        let result = ring.append(packets.iter().flat_map(|packet| &packet.instances));
        if count > 16 {
            assert_eq!(
                result.unwrap_err().to_string(),
                "particle upload exceeds native ring"
            );
            assert!(Arc::ptr_eq(&previous, &ring.allocation));
        } else {
            let length = count as u64 * 48;
            let offset = if cursor + length >= 16 * 48 {
                0
            } else {
                cursor
            };
            assert_eq!(result.unwrap(), offset);
            assert_eq!(Arc::ptr_eq(&previous, &ring.allocation), offset != 0);
            if offset == 0 {
                bytes.clear();
                known.clear();
            }
            bytes.truncate(offset as usize);
            for record in packets.iter().flat_map(|packet| &packet.instances) {
                let at = bytes.len() as u64;
                bytes.extend_from_slice(&expected::instance(record));
                known.push(at..at + 36);
            }
            cursor = offset + length;
        }
        assert_eq!(ring.cursor, cursor);
        let allocation = ring.allocation.lock().unwrap();
        assert_eq!(allocation.size, 16 * 48);
        assert_eq!(allocation.bytes, bytes);
        assert_eq!(allocation.known, known);
    }
}

fn packing_upload_refusal_and_retry(device: &wgpu::Device, program: &Program) {
    let old_arena = gpu::Arena::new(device);
    let arena = gpu::Arena::new(device);
    let plan = program
        .packing
        .as_ref()
        .expect("ordinary fixture has a copy plan");
    let big_count = (device.limits().max_buffer_size / 36 + 1) as usize;
    let capacity = (big_count + 1) * 48;
    let mut old_ring = InstanceRing::new(capacity as u64);
    let mut ring = InstanceRing::new(capacity as u64);
    let mut old = OrdinaryUploads::default();
    let mut new = packing::Uploads::default();
    let quad = bytemuck::cast_slice(&particle_shader::QUAD);
    for (phase, count) in [3, big_count, 2, big_count, 1].into_iter().enumerate() {
        let p = packet(phase, 0, "Fixture", count);
        let start = old_ring.append(p.instances.iter()).unwrap();
        let new_start = ring.append(p.instances.iter()).unwrap();
        assert_eq!(start, new_start);
        old.begin(start, old_ring.cursor);
        new.begin(start, ring.cursor);
        let a = old.packet(
            &old_arena,
            &program.layouts,
            &old_ring,
            quad,
            start,
            count as u32,
        );
        let b = match new.packet(&arena, plan, &ring, quad, start, count as u32) {
            packing::Attempt::Unproved => panic!("ordinary writer proof, phase {phase}"),
            packing::Attempt::Handled(b) => b,
        };
        assert_eq!(a.is_none(), count == big_count);
        assert_eq!(a.is_some(), b.is_some());
        if let (Some(a), Some(b)) = (a, b) {
            assert_eq!(a.layouts, b.layouts);
            assert_eq!(a.bytes, b.bytes);
            for (a, b) in a.vertices.iter().zip(&b.vertices) {
                assert_eq!((a.offset, a.size), (b.offset, b.size));
            }
        }
        let a = old_arena.vertices(&[0x5a; 16]).unwrap();
        let b = arena.vertices(&[0x5a; 16]).unwrap();
        assert_eq!((a.offset, a.size), (b.offset, b.size));
    }
    let p = packet(0, 0, "Fixture", 1);
    let start = ring.append(p.instances.iter()).unwrap();
    new.begin(start, ring.cursor);
    {
        let mut allocation = ring.allocation.lock().unwrap();
        allocation.known.last_mut().unwrap().end -= 1;
    }
    let mut narrow = program.layouts.clone();
    narrow[1]
        .attributes
        .retain(|a| a.offset + a.format.size() <= 32);
    let narrow_plan = packing::Plan::new(&narrow).unwrap();
    assert!(matches!(
        new.packet(&arena, &narrow_plan, &ring, quad, start, 1),
        packing::Attempt::Unproved
    ));
    old.begin(start, ring.cursor);
    assert!(
        old.packet(&old_arena, &narrow, &ring, quad, start, 1)
            .is_some()
    );
}
