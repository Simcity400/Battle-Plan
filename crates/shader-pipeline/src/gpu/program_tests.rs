use super::*;

#[test]
#[ignore = "requires a GPU"]
fn successful_program_matches_fresh_multiframe_bytes_offsets_and_queued_pixels() {
    use crate::{Buffer, Compiled, Shader};
    crate::tests::initialize_runtime_tables();
    let (device, queue) = crate::tests::device();
    let shader = |wgsl: &str, bindings: &[u32]| Shader {
        wgsl: wgsl.into(),
        independent_draws: false,
        buffers: bindings
            .iter()
            .map(|binding| Buffer {
                binding: *binding,
                size: 32,
                reads: std::iter::once(0..16).collect(),
            })
            .collect(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: Vec::new(),
        clock: Default::default(),
    };
    let compiled = Compiled {
        vertex: shader(
            r#"
                struct Block { value: vec4<f32>, unused: vec4<f32> }
                @group(0) @binding(0) var<uniform> scene: Block;
                @group(0) @binding(2) var<uniform> world: Block;
                struct Out { @builtin(position) position: vec4<f32>, @location(0) colour: vec4<f32> }
                @vertex fn main(@builtin(vertex_index) index: u32) -> Out {
                    var points = array<vec2<f32>,3>(vec2<f32>(-1,-1),vec2<f32>(3,-1),vec2<f32>(-1,3));
                    var out: Out;
                    out.position=vec4<f32>(points[index],0,1);
                    out.colour=scene.value + world.value;
                    return out;
                }
            "#,
            &[0, 2],
        ),
        pixel: shader(
            r#"
                struct Block { value: vec4<f32>, unused: vec4<f32> }
                @group(0) @binding(3) var<uniform> userdata: Block;
                @group(0) @binding(5) var<uniform> grid: Block;
                @fragment fn main(@location(0) colour: vec4<f32>) -> @location(0) vec4<f32> {
                    return colour * userdata.value + grid.value;
                }
            "#,
            &[3, 5],
        ),
    };
    let program = content_reader_gui::pdxshader::program::read_with(b"retained.shader", &|path| {
        Some(if path == b"retained.shader" {
            b"Effect Retained { }".to_vec()
        } else {
            Vec::new()
        })
    })
    .unwrap();
    let state = super::State::authored(&program, &program.techniques[0]).unwrap();
    let mut pass = super::Pass::packed(&compiled, &state, wgpu::TextureFormat::Rgba8Unorm, 1);
    pass.vertices.clear();
    pass.primitive.cull_mode = None;
    let pipeline = super::Pipeline::for_pass(
        &device,
        &compiled,
        &program,
        &program.techniques[0],
        &state,
        &pass,
    )
    .unwrap();
    let (fresh_arena, retained_arena) = (Arena::new(&device), Arena::new(&device));
    let mut held: [Option<BindingProgram>; 2] = [None, None];
    let mut captures = Vec::new();
    for frame in 0..7 {
        let order = if frame % 2 == 0 { [0, 1, 0] } else { [1, 0, 1] };
        let values: Vec<_> = order
            .iter()
            .enumerate()
            .map(|(draw, identity)| {
                let scene = [0.05 * frame as f32; 4];
                let world = [0.1 * (*identity as f32 + draw as f32 + 1.); 4];
                let userdata = [0.8 - 0.05 * draw as f32; 4];
                [scene, world, userdata, [0.01; 4]]
            })
            .collect();
        if frame == 2 {
            let filler = vec![0x42; 4 << 20];
            let mut offsets = [0];
            for arena in [&fresh_arena, &retained_arena] {
                arena
                    .constants(&[(&filler, filler.len() as u64)], &mut offsets)
                    .unwrap();
            }
        }
        if frame == 4 {
            for arena in [&fresh_arena, &retained_arena] {
                arena.reset();
                arena.reset();
            }
        }
        let mut fresh = Vec::new();
        let mut replayed = Vec::new();
        for (&identity, value) in order.iter().zip(&values) {
            let bytes = value.map(|value| {
                let mut bytes = bytemuck::bytes_of(&value).to_vec();
                if frame % 3 != 0 {
                    bytes.extend(
                        [0_u32, 0x80000000, 0x7fc01234, 0xffc05678]
                            .into_iter()
                            .flat_map(u32::to_le_bytes),
                    );
                }
                bytes
            });
            let data = |slot| match slot {
                0 => Some(bytes[0].as_slice()),
                2 => Some(bytes[1].as_slice()),
                3 => Some(bytes[2].as_slice()),
                5 => Some(bytes[3].as_slice()),
                _ => None,
            };
            fresh.push(pipeline.bind_with(&fresh_arena, data, |_| None).unwrap());
            let bindings = match &held[identity] {
                Some(program) => program.bind_with(&retained_arena, data).unwrap(),
                None => {
                    let (bindings, program) = pipeline
                        .bind_program_with(&retained_arena, data, |_| None)
                        .unwrap();
                    held[identity] = Some(program);
                    bindings
                }
            };
            replayed.push(bindings);
            let draw = held[identity].as_ref().unwrap();
            assert_eq!(
                fresh.last().unwrap().preparation_proof(),
                replayed.last().unwrap().preparation_proof()
            );
            assert!(
                fresh
                    .last()
                    .unwrap()
                    .same_resources(replayed.last().unwrap())
            );
            assert_eq!(
                draw.constant_reads()
                    .iter()
                    .map(|spec| spec.binding)
                    .collect::<Vec<_>>(),
                [0, 2, 3, 5]
            );
            let expected = fresh.last().unwrap();
            let actual = replayed.last().unwrap();
            assert_eq!(
                &expected.offsets[..expected.count],
                &actual.offsets[..actual.count],
                "ordinary frame staging keeps identical dynamic offsets"
            );
            for missing_slot in [0, 2, 3, 5] {
                let missing = |slot| {
                    if slot == missing_slot {
                        None
                    } else {
                        data(slot)
                    }
                };
                assert_eq!(
                    pipeline.bind_with(&fresh_arena, missing, |_| None).err(),
                    draw.bind_with(&retained_arena, missing).err()
                );
            }
        }
        let enqueue = |arena: &Arena, draws: &[Bindings]| {
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("retained in-flight pixels"),
                size: wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("retained in-flight readback"),
                size: 256 * 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let view = target.create_view(&Default::default());
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                for (index, draw) in draws.iter().enumerate() {
                    pass.set_scissor_rect(index as u32 * 5, 0, 5, 16);
                    draw.bind(&mut pass);
                    pass.draw(0..3, 0..1);
                }
            }
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: None,
                    },
                },
                target.size(),
            );
            arena.flush(&queue);
            queue.submit([encoder.finish()]);
            arena.reset();
            read
        };
        captures.push((
            enqueue(&fresh_arena, &fresh),
            enqueue(&retained_arena, &replayed),
        ));
    }
    for (expected, actual) in &captures {
        for read in [expected, actual] {
            read.slice(..)
                .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        }
    }
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    for (frame, (expected, actual)) in captures.iter().enumerate() {
        let pixels = |read: &wgpu::Buffer| {
            read.slice(..)
                .get_mapped_range()
                .unwrap()
                .chunks_exact(256)
                .flat_map(|row| row[..64].iter().copied())
                .collect::<Vec<_>>()
        };
        let (expected, actual) = (pixels(expected), pixels(actual));
        assert_eq!(expected, actual, "frame {frame}");
        assert!(actual.iter().any(|byte| *byte != 0));
    }
}

#[test]
fn preparation_proof_keeps_signed_zero_nan_payloads_and_physical_zero_tails() {
    let bits = [0_u32, 0x80000000, 0x7fc01234, 0xffc05678];
    let bytes = bits
        .iter()
        .flat_map(|bits| bits.to_le_bytes())
        .collect::<Vec<_>>();
    let specs = [
        Buffer {
            binding: 2,
            size: 32,
            reads: std::iter::once(0..16).collect(),
        },
        Buffer {
            binding: 3,
            size: 8,
            reads: std::iter::once(0..8).collect(),
        },
    ];
    let proof = preparation_bytes(&specs, &[(&bytes, 32), (&bytes, 8)]);
    assert_eq!(proof[0].0, 2);
    assert_eq!(&proof[0].1[..16], bytes);
    assert_eq!(&proof[0].1[16..], [0; 16]);
    assert_eq!(proof[1], (3, bytes));
}
