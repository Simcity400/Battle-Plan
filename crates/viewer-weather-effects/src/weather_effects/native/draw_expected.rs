use super::*;

pub(super) fn constants(packet: &Packet) -> ([u8; 16], [u8; 96]) {
    let mut scale = [0; 16];
    for (target, value) in scale.chunks_exact_mut(4).zip(packet.scale) {
        target.copy_from_slice(&value.to_bits().to_le_bytes());
    }
    let mut object = [0; 96];
    for (target, value) in object[..64]
        .chunks_exact_mut(4)
        .zip(packet.world.iter().flatten())
    {
        target.copy_from_slice(&value.to_bits().to_le_bytes());
    }
    object[72..76].copy_from_slice(&(packet.tiles[0] as f32).to_bits().to_le_bytes());
    object[76..80].copy_from_slice(&(packet.tiles[1] as f32).to_bits().to_le_bytes());
    object[80..84].copy_from_slice(&packet.time.to_bits().to_le_bytes());
    (scale, object)
}

pub(super) fn instance(instance: &Instance) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in instance.position_size.iter().chain(&instance.rotation) {
        bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    bytes.extend_from_slice(&instance.tile);
    bytes.extend_from_slice(&instance.color);
    bytes.extend_from_slice(&instance.tail);
    assert_eq!(bytes.len(), 48);
    bytes
}

fn layouts(index: usize) -> Vec<gpu::VertexLayout> {
    use wgpu::VertexFormat::{Float32x2, Float32x3, Float32x4, Uint8x4, Unorm8x4};
    let attribute = |format, offset, shader_location| wgpu::VertexAttribute {
        format,
        offset,
        shader_location,
    };
    if matches!(index, 2 | 3) {
        return vec![gpu::VertexLayout {
            stride: 28,
            step: wgpu::VertexStepMode::Vertex,
            attributes: vec![
                attribute(Float32x3, 0, 0),
                attribute(Float32x2, 12, 1),
                attribute(Uint8x4, 20, 2),
                attribute(Unorm8x4, 24, 3),
            ],
        }];
    }
    let mut instances = vec![
        attribute(Float32x4, 0, 1),
        attribute(Float32x3, 16, 2),
        attribute(Uint8x4, 28, 3),
        attribute(Unorm8x4, 32, 4),
    ];
    if index == 1 {
        instances.reverse();
    }
    vec![
        gpu::VertexLayout {
            stride: 8,
            step: wgpu::VertexStepMode::Vertex,
            attributes: vec![attribute(Float32x2, 0, 0)],
        },
        gpu::VertexLayout {
            stride: 48,
            step: wgpu::VertexStepMode::Instance,
            attributes: instances,
        },
    ]
}

pub(super) struct Draw {
    pub packet: usize,
    pub name: &'static str,
    pub trail: bool,
    pub vertices: u32,
    pub instances: u32,
    pub state: State,
}

pub(super) struct Frame {
    pub draws: Vec<Draw>,
    pub final_state: State,
}

pub(super) fn frame(
    effect: &Arc<Effect>,
    pages: &[wgpu::TextureView],
    inputs: Inputs<'_>,
    packets: &[DrawPacket],
) -> Frame {
    let mut state = State::default();
    state.inherit(inputs.retained);
    for stage in [Stage::Vertex, Stage::Pixel] {
        state
            .stream
            .bind_partial_constants(0, stage, Some(inputs.frame), &[0..348, 352..408]);
        state.stream.bind_constants(
            1,
            stage,
            Some(bytemuck::bytes_of(&matrix(Transform::IDENTITY))),
        );
    }
    state
        .stream
        .bind_constants(2, Stage::Pixel, Some(inputs.grid));
    state
        .stream
        .bind_depth_stencil(content_reader_gui::pdxshader::DepthStencilState {
            depth_write_mask: 0,
            ..Default::default()
        });
    let samplers = viewer_scene_base::mesh_shader::map_samplers();
    for (slot, texture) in inputs.textures.iter().enumerate() {
        let slot = slot as u32 + 4;
        state.stream.bind_texture(
            slot,
            Stage::Pixel,
            Some((*texture).clone()),
            samplers.get(&binding(slot, Stage::Pixel)),
        );
    }
    let quad: Arc<[u8]> = bytemuck::cast_slice(&[[0_f32, 0.], [0., 1.], [1., 0.], [1., 1.]]).into();
    let mut actor = None;
    let mut cursor = 0_u64;
    let mut offset = 0;
    let mut ring = Arc::new(Mutex::new(Partial {
        bytes: Vec::new(),
        size: 16 * 48,
        known: Vec::new(),
    }));
    let mut draws = Vec::new();
    for (at, packet) in packets.iter().enumerate() {
        assert!(packet.views.contains(0));
        let key = (packet.registration, packet.actor);
        let (scale, object) = constants(&packet.constants);
        if actor != Some(key) {
            actor = Some(key);
            for stage in [Stage::Vertex, Stage::Pixel] {
                state.stream.bind_partial_constants(
                    2,
                    stage,
                    Some(&scale),
                    std::slice::from_ref(&(0..8)),
                );
            }
            state.bind_vertex(0, 8, quad.clone());
            let records: Vec<_> = packets[at..]
                .iter()
                .take_while(|p| (p.registration, p.actor) == key && !p.descriptor.trail)
                .flat_map(|p| p.instances.iter())
                .collect();
            let length = records.len() as u64 * 48;
            assert!(length <= 16 * 48);
            if cursor + length >= 16 * 48 {
                cursor = 0;
            }
            offset = cursor;
            if cursor == 0 {
                ring = Arc::new(Mutex::new(Partial {
                    bytes: Vec::new(),
                    size: 16 * 48,
                    known: Vec::new(),
                }));
            }
            let mut bytes = ring.lock().unwrap();
            bytes.bytes.truncate(cursor as usize);
            for record in records {
                let start = bytes.bytes.len() as u64;
                bytes.bytes.extend_from_slice(&instance(record));
                bytes.known.push(start..start + 36);
            }
            cursor += length;
        }
        if packet.instances.is_empty() {
            continue;
        }
        let trail = packet.descriptor.trail.then(|| {
            particle_shader::trail::vertices(
                &packet.instances,
                [0., 0., 2.],
                packet.constants.scale[1],
                packet.descriptor.billboard,
            )
            .unwrap()
        });
        if trail.as_ref().is_some_and(Vec::is_empty) {
            continue;
        }
        let (name, selected) = match (
            packet.descriptor.shader.as_str(),
            packet.descriptor.trail,
            packet.descriptor.billboard,
        ) {
            ("Fixture", false, true) => ("Fixture", Some(0)),
            ("Alternate", false, true) => ("Alternate", Some(1)),
            ("Fixture", true, true) => ("FixtureTrail", Some(2)),
            ("Fixture", true, false) => ("FixtureTrailNoBillboard", Some(3)),
            ("UnknownScale", false, true) => ("UnknownScale", Some(4)),
            ("Missing", false, true) => ("Missing", None),
            ("Missing", true, true) => ("MissingTrail", None),
            _ => panic!("unlisted fixture declaration"),
        };
        if let Some(index) = selected {
            let selection = effect.select(name).unwrap().unwrap();
            assert_eq!(selection.index, index);
            state.stream.select_program(&selection, &layouts(index));
        } else {
            state.stream.bind_program(None, Vec::new());
        }
        state.stream.bind_blend(None, wgpu::ColorWrites::ALL);
        state
            .stream
            .bind_rasterizer(content_reader_gui::pdxshader::RasterizerState {
                cull_mode: if selected.is_some() { 0 } else { 2 },
                ..Default::default()
            });
        for stage in [Stage::Vertex, Stage::Pixel] {
            state.stream.bind_partial_constants(
                2,
                stage,
                Some(&scale),
                std::slice::from_ref(&(0..8)),
            );
        }
        state.stream.bind_partial_constants(
            3,
            Stage::Vertex,
            Some(&object),
            std::slice::from_ref(&(0..84)),
        );
        state
            .stream
            .bind_texture(0, Stage::Pixel, Some(pages[packet.page].clone()), None);
        let (vertices, instances) = if let Some(vertices) = &trail {
            let bytes: Vec<u8> = bytemuck::cast_slice(vertices).to_vec();
            let length = bytes.len() as u64;
            state.bind_partial_vertex(
                0,
                28,
                Arc::new(Mutex::new(Partial {
                    bytes,
                    size: 16 * 56 * 28,
                    known: std::iter::once(0..length).collect(),
                })),
                0,
            );
            (vertices.len() as u32, 1)
        } else {
            state.bind_vertex(0, 8, quad.clone());
            state.bind_partial_vertex(1, 48, ring.clone(), offset);
            offset += packet.instances.len() as u64 * 48;
            (4, packet.instances.len() as u32)
        };
        draws.push(Draw {
            packet: at,
            name,
            trail: trail.is_some(),
            vertices,
            instances,
            state: state.snapshot(),
        });
    }
    Frame {
        draws,
        final_state: state.snapshot(),
    }
}
