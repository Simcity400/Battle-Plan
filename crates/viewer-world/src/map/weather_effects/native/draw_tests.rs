//! Reference/optimized draw proofs, independent of the presentation simulation.
use super::*;
use crate::map::shader_scene::State;
use shader_pipeline::{Buffer, Compiled, ScalarKind, Shader, TextureKind, VertexInput};

fn packet(frame: usize, actor: usize, shader: &str, count: usize) -> DrawPacket {
    let emitter = super::super::tests::activation_test_emitter();
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
fn stack_constants_match_packet_uploads_bit_for_bit() {
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
        let old = p.constants.buffers();
        let new = DrawConstants::new(&p.constants);
        for stage in [Stage::Vertex, Stage::Pixel] {
            assert_eq!(old[&binding(2, stage)], bytemuck::bytes_of(&new.scale));
        }
        assert_eq!(old[&binding(3, Stage::Vertex)], new.object);
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
    let source = Arc::new(installed_content_core::pdxshader::program::read_with(b"weather-proof.shader", &|path| {
        Some(if path == b"weather-proof.shader" {
            b"RasterizerState Visible { CullMode = none } Effect Fixture { RasterizerState = Visible } Effect Alternate { RasterizerState = Visible } Effect FixtureTrail { RasterizerState = Visible } Effect FixtureTrailNoBillboard { RasterizerState = Visible } Effect UnknownScale { RasterizerState = Visible }".to_vec()
        } else { Vec::new() })
    }).unwrap());
    let shader = |wgsl: &str| Shader {
        wgsl: wgsl.into(),
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
        // An empty packet still establishes actor binds, without breaking the prior draw.
        packets.push(packet(frame, actor, "Missing", 0));
        let mut p = packet(frame, actor, "Fixture", frame % 4);
        p.descriptor.trail = true;
        p.descriptor.billboard = frame.is_multiple_of(2);
        packets.push(p);
    }
    // Missing techniques exercise the original path through both kinds of inherited layouts.
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
fn weather_draw_reference_and_reused_uploads_bindings_match_many_frames() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let (_serial, device, queue) = viewer_foundation::map::test_support::gpu::device();
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
    let targets = Targets::new(&device, [16, 16], [16, 16], 1);
    let arena = gpu::Arena::new(&device);
    let frame_bytes = [0_u8; 416];
    let grid = [0_u8; 16];
    let projection = matrix(Transform::IDENTITY);
    let mut prepared = 0;
    let mut reused = 0;
    let mut refused = 0;
    let mut nonzero_pixels = 0;
    for replacement in [false, true] {
        let mut renderer = Renderer::new(&device, effect(replacement), pages.clone(), 1, 16);
        let mut retained: [State; 2] = Default::default();
        for frame in 0..48 {
            let packets = if frame % 13 == 12 {
                Vec::new()
            } else {
                frame_packets(frame)
            };
            let mut outcomes = Vec::new();
            // Alternate path order so neither depends on a cold pipeline or arena cache.
            for reference in if frame.is_multiple_of(2) {
                [true, false]
            } else {
                [false, true]
            } {
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("particle proof clear"),
                        color_attachments: &[Some(
                            targets.scene_attachment(wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)),
                        )],
                        depth_stencil_attachment: Some(
                            targets.depth_attachment(wgpu::LoadOp::Clear(1.)),
                        ),
                        ..Default::default()
                    });
                }
                renderer.trace = Some(Vec::new());
                let old_refused = renderer.retained.stream.refused.clone();
                let old_cache_refused = renderer.retained.stream.cache.refused.clone();
                renderer.reference = reference;
                let result = renderer.draw(
                    &arena,
                    &mut encoder,
                    &targets,
                    projection,
                    [0., 0., 2.],
                    0,
                    Inputs {
                        frame: &frame_bytes,
                        grid: &grid,
                        textures: std::array::from_fn(|_| &pages[0]),
                        retained: &mut retained[usize::from(reference)],
                    },
                    &packets,
                );
                let errors = delta(&old_refused, &renderer.retained.stream.refused);
                let cache_errors =
                    delta(&old_cache_refused, &renderer.retained.stream.cache.refused);
                let mut final_state = State::default();
                final_state.inherit(&renderer.retained);
                let trace = renderer.trace.take().unwrap();
                let result = result.err().map(|e| e.to_string());
                arena.flush(&queue);
                queue.submit([encoder.finish()]);
                let pixels =
                    gpu_contract::read_texture(&device, &queue, targets.scene_texture(), 8)
                        .unwrap();
                outcomes.push((
                    reference,
                    result,
                    errors,
                    cache_errors,
                    trace,
                    final_state,
                    pixels,
                ));
            }
            let a = &outcomes[0];
            let b = &outcomes[1];
            assert_eq!(a.1, b.1, "frame {frame}, replacement {replacement}");
            assert_eq!(a.2, b.2);
            assert_eq!(a.3, b.3);
            refused += a.2.values().sum::<u64>();
            assert_eq!(a.4.len(), b.4.len());
            for (index, (a, b)) in a.4.iter().zip(&b.4).enumerate() {
                assert_eq!(a.layouts, b.layouts, "frame {frame}, draw {index}");
                assert_eq!(a.bytes, b.bytes, "frame {frame}, draw {index}");
                assert!(
                    a.bindings.same_resources(&b.bindings),
                    "frame {frame}, draw {index}"
                );
                assert!(
                    a.retained.same_bindings(&b.retained),
                    "frame {frame}, draw {index}"
                );
                prepared += usize::from(a.prepared || b.prepared);
                reused += usize::from(a.reused || b.reused);
            }
            assert!(a.5.same_bindings(&b.5));
            assert!(retained[0].same_bindings(&retained[1]));
            assert_eq!(a.6, b.6, "frame {frame}, replacement {replacement}");
            nonzero_pixels += a.6.iter().filter(|&&byte| byte != 0).count();
            arena.reset();
        }
    }
    assert!(prepared > 500, "ordinary per-actor uploads were exercised");
    assert!(reused > 500, "within-view bindings were reused");
    assert!(refused > 0, "unknown constant reads were refused");
    assert!(
        nonzero_pixels > 0,
        "pixel comparison must include visible geometry"
    );
}
