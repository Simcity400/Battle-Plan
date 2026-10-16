//! Candidate packets from the existing presentation simulator. Shader bytes,
//! declarations and constants are native; historical simulation/order is separate.
use super::{Emitter, Simulation};
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{mesh::Transform, particles::RenderDescriptor, pdxshader::program::Stage};
use shader_pipeline::{binding, gpu, library::Effect};
use std::collections::BTreeSet;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};
use {
    crate::map::{
        particle_shader::{self, Instance, Packet},
        shader_scene::Partial,
    },
    viewer_foundation::map::gpu_contract::{self, Targets},
};

pub struct Inputs<'a> {
    pub frame: &'a [u8],
    pub grid: &'a [u8],
    pub textures: [&'a wgpu::TextureView; 10],
    pub retained: &'a mut crate::map::shader_scene::State,
}

/// The wrap views a packet draws in, one flag a view in the frame's order, as many as the frame
/// has: a free camera can reach any number of map copies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ViewMask {
    /// Every view.
    All,
    /// Whether each view draws the packet; views past the end do not.
    Some(Vec<bool>),
}

impl ViewMask {
    pub(super) fn contains(&self, view: usize) -> bool {
        match self {
            Self::All => true,
            Self::Some(views) => views.get(view).copied().unwrap_or(false),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        matches!(self, Self::Some(views) if !views.contains(&true))
    }
}

pub(super) struct DrawPacket {
    page: usize,
    registration: usize,
    pub actor: [usize; 4],
    /// Prepared admission in the frame's ordered native camera envelopes.
    pub views: ViewMask,
    pub(super) descriptor: RenderDescriptor,
    constants: Packet,
    pub instances: Vec<Instance>,
}

fn matrix(transform: Transform) -> [[f32; 4]; 4] {
    let [a, b, c] = transform.basis;
    let t = transform.translation;
    [
        [a[0], a[1], a[2], 0.],
        [b[0], b[1], b[2], 0.],
        [c[0], c[1], c[2], 0.],
        [t[0], t[1], t[2], 1.],
    ]
}

impl DrawPacket {
    fn new(e: &Emitter, authored: f32, mut world: Transform, time: f32) -> Self {
        let scale = authored
            * if e.system_local_space {
                particle_shader::world_scale(world.basis)
            } else {
                1.
            };
        if e.local_space {
            // Normalize each column separately; a zero column stays zero.
            for column in &mut world.basis {
                let length =
                    (column[0] * column[0] + column[1] * column[1] + column[2] * column[2]).sqrt();
                if length != 0. {
                    let reciprocal = 1. / length;
                    for value in column {
                        *value *= reciprocal;
                    }
                }
            }
        } else {
            world = Transform::IDENTITY;
        }
        Self {
            page: e.page,
            registration: e.registration,
            actor: [0; 4],
            views: ViewMask::All,
            descriptor: e.render.clone(),
            constants: Packet {
                world: matrix(world),
                scale: [scale, if e.system_local_space { scale } else { 1. }],
                tiles: e.render.tiles,
                time,
            },
            instances: Vec::new(),
        }
    }

    pub fn weather(e: &Emitter, sim: &Simulation, anchor: [f32; 3], alpha: f32) -> Self {
        let mut out = Self::new(
            e,
            e.scale,
            Transform {
                translation: anchor,
                ..Transform::IDENTITY
            },
            sim.age,
        );
        for p in &sim.particles {
            let t = p.age / p.life;
            let position = if e.local_space {
                p.position
            } else {
                std::array::from_fn(|i| p.position[i] + anchor[i] - sim.anchor[i])
            };
            let mut color = std::array::from_fn(|i| {
                e.color[i].animate(p.color[i], t).clamp(0., 255.).trunc() as u8
            });
            // Presentation camera fade is retained independently of shader conversion.
            color[3] = (f32::from(color[3]) * alpha).round() as u8;
            out.instances.push(Instance {
                position_size: [
                    position[0],
                    position[1],
                    position[2],
                    e.size.animate(p.size, t).max(0.),
                ],
                rotation: p.rotation,
                tile: p.tile,
                color,
                tail: [0; 12],
            });
        }
        out
    }

    pub fn entity(e: &Emitter, scale: f32, event: &crate::map::entity_animation::Event) -> Self {
        // Copy the runtime's world matrix, independent of view.
        Self::new(e, scale, event.transform, event.age - e.start.base)
    }
}

///Viewer-owned presentation histories. A newly observed event starts empty;
///its historical RNG, actor enqueue order and pre-observation motion are not recorded.
#[derive(Default)]
pub(crate) struct EntitySimulations(
    BTreeMap<(String, crate::map::entity_animation::EventKey, usize), Simulation>,
);

impl EntitySimulations {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Advance once before scene lights and draws through the same path for every entity producer.
    /// Retain a kept event's light until its finished particle runtime has no living subsystem.
    pub fn prepare(
        &mut self,
        entities: &BTreeMap<String, (f32, Vec<Emitter>)>,
        events: &[crate::map::entity_animation::Event],
    ) -> io::Result<()> {
        let mut seen = BTreeSet::new();
        for event in events {
            let Some(name) = &event.definition.particle else {
                continue;
            };
            let Some((scale, emitters)) = entities.get(name) else {
                continue;
            };
            for (ordinal, emitter) in emitters.iter().enumerate() {
                let key = (name.clone(), event.key, ordinal);
                // Frame-local invariant; later observations reuse the runtime below.
                if !seen.insert(key.clone()) {
                    return Err(io::Error::other(format!(
                        "Map shader particles: 1 refused actor {name}: duplicate presentation identity {:?}",
                        event.key
                    )));
                }
                let sim = self.0.entry(key).or_insert_with(|| {
                    Simulation::cold(
                        emitter,
                        u64::from(event.seed) ^ ((ordinal as u64) << 32),
                        event.transform.translation,
                        event.age,
                    )
                });
                let space = Some(super::motion::Space::new(
                    emitter.local_space,
                    emitter.system_local_space,
                    *scale,
                    event.transform,
                ));
                sim.advance_event(emitter, event.age, event.emission_end, space);
            }
        }
        self.0
            .retain(|key, sim| seen.contains(key) && (sim.emitting || !sim.particles.is_empty()));
        Ok(())
    }

    /// Every runtime's identity, age, seed, emission state, accumulator and particles, for
    /// comparing two sets of simulations stepped alike.
    #[cfg(test)]
    pub(crate) fn presentation_state(&self) -> String {
        self.0
            .iter()
            .map(|((name, key, ordinal), sim)| {
                // The pass namespace differs between passes; the rest names the runtime.
                format!(
                    "{name} {:?} {ordinal} age={} time={} seed={} emitting={} accumulator={} particles={:?}\n",
                    key.without_pass(),
                    sim.age,
                    sim.time,
                    sim.seed,
                    sim.emitting,
                    sim.accumulator,
                    sim.particles
                        .iter()
                        .map(|p| (p.age, p.life, p.position, p.size, p.rotation))
                        .collect::<Vec<_>>()
                )
            })
            .collect()
    }

    pub fn living(&self, event: &crate::map::entity_animation::Event) -> bool {
        let Some(name) = &event.definition.particle else {
            return false;
        };
        self.0
            .range((name.clone(), event.key, 0)..=(name.clone(), event.key, usize::MAX))
            .any(|(_, sim)| !sim.particles.is_empty())
    }

    pub(super) fn packets(
        &self,
        entities: &BTreeMap<String, (f32, Vec<Emitter>)>,
        events: &[crate::map::entity_animation::Event],
        packets: &mut Vec<DrawPacket>,
    ) {
        // A hidden event's runtime keeps simulating; only its particles are not submitted.
        for (actor, event) in events.iter().enumerate().filter(|(_, event)| !event.hidden) {
            let Some(name) = &event.definition.particle else {
                continue;
            };
            let Some((scale, emitters)) = entities.get(name) else {
                continue;
            };
            for (ordinal, emitter) in emitters.iter().enumerate() {
                let key = (name.clone(), event.key, ordinal);
                let Some(sim) = self.0.get(&key) else {
                    continue;
                };
                let mut packet = DrawPacket::entity(emitter, *scale, event);
                packet.actor = [1, actor, 0, 0];
                packet.constants.time = sim.age;
                for particle in &sim.particles {
                    let t = particle.age / particle.life;
                    let position = particle.position;
                    packet.instances.push(Instance {
                        position_size: [
                            position[0],
                            position[1],
                            position[2],
                            emitter.size.animate(particle.size, t).max(0.),
                        ],
                        rotation: particle.rotation,
                        tile: particle.tile,
                        color: std::array::from_fn(|i| {
                            emitter.color[i]
                                .animate(particle.color[i], t)
                                .clamp(0., 255.)
                                .trunc() as u8
                        }),
                        tail: [0; 12],
                    });
                }
                packets.push(packet);
            }
        }
    }
}

fn submission_order(packets: &[DrawPacket], view: usize) -> Vec<&DrawPacket> {
    let mut ordered: Vec<_> = packets
        .iter()
        .filter(|packet| packet.views.contains(view))
        .collect();
    // Submit each actor's ordinary packets before that actor's trails.
    ordered.sort_by_key(|packet| (packet.registration, packet.actor, packet.descriptor.trail));
    ordered
}

/// Physical bytes of a native vertex allocation of `capacity` records.
fn allocation_size(capacity: usize, stride: usize) -> io::Result<u64> {
    capacity
        .checked_mul(stride)
        .filter(|n| *n <= i32::MAX as usize)
        .map(|n| n as u64)
        .ok_or_else(|| io::Error::other("particle native vertex allocation overflow"))
}

fn allocation(size: u64) -> Arc<Mutex<Partial>> {
    Arc::new(Mutex::new(Partial {
        bytes: Vec::new(),
        size,
        known: Vec::new(),
    }))
}

/// Enabled D3D11 particle rings discard at or beyond capacity; otherwise append with NO_OVERWRITE to
/// the same allocation. Reset the cursor for each view.
struct InstanceRing {
    allocation: Arc<Mutex<Partial>>,
    size: u64,
    cursor: u64,
}

impl InstanceRing {
    fn new(size: u64) -> Self {
        Self {
            allocation: allocation(size),
            size,
            cursor: 0,
        }
    }

    /// Upload every nonempty ordinary packet of an actor at once and
    /// binds each packet's offset into it. The final 12 instance bytes
    /// have no established writer and stay unknown. Returns the upload's offset.
    fn append<'a>(&mut self, instances: impl Iterator<Item = &'a Instance>) -> io::Result<u64> {
        let instances: Vec<_> = instances.collect();
        let length = (instances.len() as u64)
            .checked_mul(48)
            .ok_or_else(|| io::Error::other("particle upload overflow"))?;
        if length > self.size {
            return Err(io::Error::other("particle upload exceeds native ring"));
        }
        if self.cursor + length >= self.size {
            self.cursor = 0;
        }
        let offset = self.cursor;
        if offset == 0 {
            // DISCARD renames the allocation; earlier binds keep the old contents.
            self.allocation = allocation(self.size);
        }
        let mut allocation = self
            .allocation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        allocation.bytes.truncate(offset as usize);
        for instance in instances {
            let at = allocation.bytes.len() as u64;
            allocation
                .bytes
                .extend_from_slice(bytemuck::bytes_of(instance));
            allocation.known.push(at..at + 36);
        }
        self.cursor += length;
        Ok(offset)
    }
}

pub(super) struct Renderer {
    effect: Arc<Effect>,
    pages: Vec<wgpu::TextureView>,
    samples: u32,
    indices: wgpu::Buffer,
    retained: crate::map::shader_scene::State,
    max_particles: usize,
    /// Per technique name and trail flag; `None` is the missing technique.
    programs: BTreeMap<(String, bool), Option<Program>>,
}

/// A selected technique's declaration and authored states.
struct Program {
    selection: shader_pipeline::library::Selection,
    layouts: Vec<gpu::VertexLayout>,
    state: gpu::State,
    raster: installed_content::pdxshader::RasterizerState,
}

impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        effect: Arc<Effect>,
        pages: Vec<wgpu::TextureView>,
        samples: u32,
        max_particles: usize,
    ) -> Self {
        let buffer = |label, contents, usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        Self {
            effect,
            pages,
            samples,
            max_particles,
            retained: Default::default(),
            programs: BTreeMap::new(),
            indices: buffer(
                "native particle indices",
                bytemuck::cast_slice(&particle_shader::INDICES),
                wgpu::BufferUsages::INDEX,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        projection: [[f32; 4]; 4],
        eye: [f32; 3],
        view: usize,
        inputs: Inputs<'_>,
        packets: &[DrawPacket],
    ) -> io::Result<()> {
        self.retained.inherit(inputs.retained);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compiled particles"),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            ..Default::default()
        });
        // Walk registered type buckets, then each actor's packets.
        // Stable grouping preserves this viewer's actor/packet enqueue order;
        // historical cross-producer actor order remains unrecorded.
        let ordered = submission_order(packets, view);
        let mut actor = None;
        let mut ordinary_offset = 0;
        let mut ring = InstanceRing::new(allocation_size(self.max_particles, 48)?);
        let trail_size = allocation_size(self.max_particles, 56 * 28)?;
        let depth = installed_content::pdxshader::DepthStencilState {
            depth_enable: true,
            depth_write_mask: 0,
            depth_function: 3,
            stencil_enable: false,
            ..Default::default()
        };
        let template = |topology| gpu::Pass {
            vertices: Vec::new(),
            primitive: wgpu::PrimitiveState {
                topology,
                ..Default::default()
            },
            depth_stencil: Some(
                gpu::depth_stencil(&depth, &Default::default(), gpu_contract::DEPTH_FORMAT).0,
            ),
            multisample: wgpu::MultisampleState {
                count: self.samples,
                ..Default::default()
            },
            targets: vec![Some(wgpu::ColorTargetState {
                format: gpu_contract::SCENE_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        };
        let [list, strip] = [
            template(wgpu::PrimitiveTopology::TriangleList),
            template(wgpu::PrimitiveTopology::TriangleStrip),
        ];
        // Bind these resources even with no actors. The next seam inherits the empty view's render state.
        {
            let stream = &mut self.retained.stream;
            for stage in [Stage::Vertex, Stage::Pixel] {
                stream.bind_partial_constants(0, stage, Some(inputs.frame), &[0..348, 352..408]);
                stream.bind_constants(1, stage, Some(bytemuck::bytes_of(&projection)));
            }
            // Restore the complete sixteen-byte light grid at PS b2.
            stream.bind_constants(2, Stage::Pixel, Some(inputs.grid));
            stream.bind_depth_stencil(depth.clone());
            let map_samplers = crate::map::mesh_shader::map_samplers();
            for (slot, texture) in inputs.textures.iter().enumerate() {
                let slot = slot as u32 + 4;
                stream.bind_texture(
                    slot,
                    Stage::Pixel,
                    Some((*texture).clone()),
                    map_samplers.get(&binding(slot, Stage::Pixel)),
                );
            }
        }
        let quad: Arc<[u8]> = bytemuck::cast_slice(&particle_shader::QUAD).into();
        let mut previous: Option<(Arc<gpu::Pipeline>, gpu::Bindings)> = None;
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        for (index, packet) in ordered.iter().enumerate() {
            let key_actor = (packet.registration, packet.actor);
            if actor != Some(key_actor) {
                actor = Some(key_actor);
                // Bind an enabled actor's scale and quad even
                //when all of its subsystem packet counts are zero.
                let scale = [packet.constants.scale[0], packet.constants.scale[1], 0., 0.];
                for stage in [Stage::Vertex, Stage::Pixel] {
                    self.retained.stream.bind_partial_constants(
                        2,
                        stage,
                        Some(bytemuck::bytes_of(&scale)),
                        std::slice::from_ref(&(0..8)),
                    );
                }
                self.retained.bind_vertex(0, 8, quad.clone());
                // Ordering puts each actor's ordinary packets first, together.
                ordinary_offset = ring.append(
                    ordered[index..]
                        .iter()
                        .take_while(|p| {
                            (p.registration, p.actor) == key_actor && !p.descriptor.trail
                        })
                        .flat_map(|p| &p.instances),
                )?;
            }
            if packet.instances.is_empty() {
                continue;
            }
            let name = packet.descriptor.technique();
            let trail = if packet.descriptor.trail {
                let vertices = particle_shader::trail::vertices(
                    &packet.instances,
                    eye,
                    packet.constants.scale[1],
                    packet.descriptor.billboard,
                )?;
                if vertices.is_empty() {
                    continue;
                }
                Some(vertices)
            } else {
                None
            };
            let program = match self.programs.entry((name.clone(), trail.is_some())) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let program = match self.effect.select(&name).map_err(io::Error::other)? {
                        Some(selection) => {
                            let compiled = self
                                .effect
                                .compiled(selection.index)
                                .map_err(io::Error::other)?;
                            let technique = &self.effect.source.techniques[selection.index];
                            Some(Program {
                                layouts: particle_shader::layouts(
                                    &compiled.vertex.inputs,
                                    trail.is_some(),
                                )?,
                                state: gpu::State::authored(&self.effect.source, technique)
                                    .map_err(io::Error::other)?,
                                raster: self.effect.source.rasterizer(technique),
                                selection,
                            })
                        }
                        None => None,
                    };
                    entry.insert(program)
                }
            };
            let buffers = packet.constants.buffers();
            let stream = &mut self.retained.stream;
            match program {
                Some(program) => {
                    stream.bind_program(Some(program.selection.clone()), program.layouts.clone());
                    stream.bind_blend(program.state.blend, program.state.write_mask);
                    stream.bind_rasterizer(program.raster.clone());
                }
                None => {
                    stream.bind_program(None, Vec::new());
                    stream.bind_blend(None, wgpu::ColorWrites::ALL);
                    stream.bind_rasterizer(Default::default());
                }
            }
            for stage in [Stage::Vertex, Stage::Pixel] {
                stream.bind_partial_constants(
                    2,
                    stage,
                    Some(&buffers[&binding(2, stage)]),
                    std::slice::from_ref(&(0..8)),
                );
            }
            stream.bind_partial_constants(
                3,
                Stage::Vertex,
                Some(&buffers[&binding(3, Stage::Vertex)]),
                std::slice::from_ref(&(0..84)),
            );
            stream.bind_texture(
                0,
                Stage::Pixel,
                Some(self.pages[packet.page].clone()),
                program
                    .as_ref()
                    .and_then(|program| program.state.samplers.get(&binding(0, Stage::Pixel))),
            );
            if let Some(vertices) = &trail {
                // Allocate maximum count times 56 bytes at stride 28, independently of this upload's size.
                // Discard the allocation before writing its initialized prefix.
                let upload: &[u8] = bytemuck::cast_slice(vertices);
                if upload.len() as u64 > trail_size {
                    return Err(io::Error::other(
                        "particle upload exceeds native vertex allocation",
                    ));
                }
                let written = Partial {
                    bytes: upload.to_vec(),
                    size: trail_size,
                    known: std::iter::once(0..upload.len() as u64).collect(),
                };
                self.retained
                    .bind_partial_vertex(0, 28, Arc::new(Mutex::new(written)), 0);
            } else {
                self.retained.bind_vertex(0, 8, quad.clone());
                self.retained
                    .bind_partial_vertex(1, 48, ring.allocation.clone(), ordinary_offset);
                ordinary_offset += (packet.instances.len() * 48) as u64;
            }
            let count =
                u32::try_from(trail.as_ref().map_or(4, Vec::len)).map_err(io::Error::other)?;
            let instances = if trail.is_some() {
                1
            } else {
                u32::try_from(packet.instances.len()).map_err(io::Error::other)?
            };
            let submission = self.retained.prepare(
                arena,
                &format!("particle {name}"),
                if trail.is_some() { &strip } else { &list },
                count,
                instances,
            )?;
            let pipeline = &submission.pass.pipeline;
            let same = previous
                .as_ref()
                .filter(|(last, _)| Arc::ptr_eq(last, pipeline))
                .map(|(_, bindings)| bindings);
            if same.is_none() {
                pass.set_pipeline(&pipeline.pipeline);
                pass.set_stencil_reference(submission.pass.stencil_reference.unwrap_or(0));
                pass.set_blend_constant(wgpu::Color::WHITE);
            }
            submission.pass.bindings.bind_after(&mut pass, same);
            for (slot, buffer) in submission.vertices.iter().enumerate() {
                pass.set_vertex_buffer(slot as u32, buffer.slice());
            }
            if trail.is_some() {
                pass.draw(0..count, 0..instances);
            } else {
                pass.draw_indexed(0..6, 0, 0..instances);
            }
            previous = Some((submission.pass.pipeline, submission.pass.bindings));
        }
        //Keep the renderer's persistent device cache while publishing the
        //bindings/vertex allocations subsequent meshes and popups inherit.
        inputs.retained.inherit(&self.retained);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settled_hulls_keep_one_simulation_per_persistent_event() {
        use crate::map::{entity_animation, models, unit_history};
        let mut rig = entity_animation::stage_rig(false);
        rig.nodes[0].clips[0].animation = None;
        rig.nodes[0].clips[0]
            .state
            .events
            .push(installed_content::mesh::EntityEvent {
                particle: Some("wake".into()),
                keep_particle: true,
                ..Default::default()
            });
        let rigs = [Arc::new(rig)];
        let mut emitter = super::super::tests::activation_test_emitter();
        emitter.life.base = 10.;
        let entities = BTreeMap::from([("wake".into(), (1., vec![emitter]))]);
        let targets = (0..2)
            .map(|part| unit_history::Target {
                part,
                occupant: u64::from(part),
                ..unit_history::test_support::target(7, unit_history::Kind::Ship)
            })
            .collect::<Vec<_>>();
        for steady in [false, true] {
            let mut history = unit_history::History::default();
            history.set(targets.clone(), Arc::default(), steady);
            let _ = history.step(
                0.,
                unit_history::test_support::GATES,
                [true; 2],
                true,
                |_| true,
            );
            let mut players = BTreeMap::new();
            let mut simulations = EntitySimulations::default();
            let mut first = Vec::new();
            let mut previous = Vec::new();
            for time in [1., 2., 3., 4., 4., 4.] {
                let placed = history.step(
                    time,
                    unit_history::test_support::GATES,
                    [true; 2],
                    true,
                    |_| true,
                );
                let mut field =
                    models::ModelField::build(&models::ModelLibrary::default(), placed, 256.);
                let mut events = Vec::new();
                models::animate_players(
                    &mut players,
                    &rigs,
                    &[models::PaletteJoint::IDENTITY],
                    &mut field,
                    time,
                    &|_| true,
                    &mut events,
                );
                simulations.prepare(&entities, &events).unwrap();
                assert_eq!(players.len(), 2);
                assert_eq!(events.len(), 2);
                assert_eq!(simulations.0.len(), 2, "one runtime per hull event");
                if first.is_empty() {
                    let mut duplicate = events.clone();
                    duplicate.push(events[0].clone());
                    let refusal = simulations.prepare(&entities, &duplicate).unwrap_err();
                    assert!(refusal.to_string().starts_with(
                        "Map shader particles: 1 refused actor wake: duplicate presentation identity"
                    ));
                    first = events.iter().map(|event| (event.key, event.age)).collect();
                }
                let mut current = Vec::new();
                for (event, &(key, age)) in events.iter().zip(&first) {
                    assert_eq!(event.key, key, "reapplying idle keeps the live event");
                    assert!((event.age - age - (time - 1.)).abs() < 1e-5);
                    let sim = &simulations.0[&("wake".into(), key, 0)];
                    assert!((sim.age - (time - 1.)).abs() < 1e-5, "no respawn");
                    current.push((sim.seed, sim.particles.len()));
                }
                if time == 4. && !previous.is_empty() {
                    assert_eq!(current, previous, "allocation retry frames do not spawn");
                }
                if time == 4. {
                    previous = current;
                }
            }
        }
    }

    #[test]
    fn fire_packet_passes_one_atlas_tile_and_neutral_tint_to_native_shader() {
        // Keep a birth tile; uploads it with the atlas
        // dimensions.
        let mut emitter = super::super::tests::activation_test_emitter();
        emitter.render.tiles = [2, 2];
        emitter.render.shader = "ParticleAdditive".into();
        let mut simulation = Simulation::cold(&emitter, 7, [0.; 3], 0.);
        simulation.tick(&emitter, 1. / 60.);
        let particle = &mut simulation.particles[0];
        particle.tile = [1, 0, 0, 0];
        particle.color = [200., 200., 200., 255.];

        let packet = DrawPacket::weather(&emitter, &simulation, [0.; 3], 1.);
        assert_eq!(packet.descriptor.technique(), "ParticleAdditive");
        assert_eq!(packet.instances.len(), 1);
        assert_eq!(packet.instances[0].tile, [1, 0, 0, 0]);
        assert_eq!(packet.instances[0].color, [200, 200, 200, 255]);
        let buffers = packet.constants.buffers();
        let object = &buffers[&binding(3, Stage::Vertex)];
        assert_eq!(&object[72..80], bytemuck::bytes_of(&[2_f32, 2.]));
    }

    #[test]
    fn canonical_packets_use_seam_carriers_without_advancing_births() {
        use viewer_foundation::map::camera::{Camera, MapDefines, MapSize};
        let size = [1200, 800];
        let camera = Camera {
            center: [5., 400.],
            height: 600.,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: viewer_foundation::map::camera::Pose::Map,
        };
        let anchor = [camera.map.x() - 5., 10., 400.];
        for local in [false, true] {
            let mut emitter = super::super::tests::activation_test_emitter();
            emitter.local_space = local;
            let mut simulation = Simulation::cold(&emitter, 7, anchor, 0.);
            // Use the production birth space, so
            // nonlocal particles already hold canonical world positions.
            simulation.advance_space(
                &emitter,
                1. / 60.,
                1.,
                Some(super::super::motion::Space::new(
                    emitter.local_space,
                    emitter.system_local_space,
                    emitter.scale,
                    Transform {
                        translation: anchor,
                        ..Transform::IDENTITY
                    },
                )),
            );
            let seed = simulation.seed;
            let age = simulation.age;
            let packet = DrawPacket::weather(&emitter, &simulation, anchor, 1.);
            let instances = packet.instances.clone();
            assert!(!instances.is_empty());
            if local {
                assert_eq!(packet.constants.world[3][..3], anchor);
            } else {
                assert_eq!(packet.constants.world, matrix(Transform::IDENTITY));
                assert!(
                    instances
                        .iter()
                        .all(|p| p.position_size[0] > camera.map.x() - 20.)
                );
            }
            for offset in [0., -camera.map.x(), camera.map.x()] {
                let mut wrapped = camera;
                wrapped.center[0] -= offset;
                let original_copy = camera
                    .project([anchor[0] + offset, anchor[1], anchor[2]], size)
                    .unwrap();
                let canonical_copy = wrapped.project(anchor, size).unwrap();
                for coordinate in 0..2 {
                    assert!((original_copy[coordinate] - canonical_copy[coordinate]).abs() < 0.01);
                }
                let _ = packet.constants.buffers();
                assert_eq!(packet.instances, instances);
            }
            assert_eq!(simulation.seed, seed);
            assert_eq!(simulation.age, age);
            assert_eq!(simulation.particles.len(), instances.len());
        }
    }

    #[test]
    fn instance_ring_appends_in_place_keeps_tails_unknown_and_discards_at_exact_capacity() {
        let instance = |value: f32| Instance {
            position_size: [value; 4],
            rotation: [0.; 3],
            tile: [0; 4],
            color: [1, 2, 3, 4],
            tail: [0; 12],
        };
        let mut ring = InstanceRing::new(4 * 48);
        assert_eq!(ring.append([instance(1.)].iter()).unwrap(), 0);
        let first = ring.allocation.clone();
        assert_eq!(ring.append([instance(2.)].iter()).unwrap(), 48);
        // NO_OVERWRITE writes the same allocation an earlier bind holds.
        assert!(Arc::ptr_eq(&first, &ring.allocation));
        {
            let written = first.lock().unwrap();
            assert_eq!(written.known, [0..36, 48..84]);
            assert_eq!(written.size, 4 * 48);
            assert_eq!(
                &written.bytes[48..84],
                &bytemuck::bytes_of(&instance(2.))[..36]
            );
        }
        ring.append([instance(3.)].iter()).unwrap();
        assert_eq!(ring.append([instance(4.)].iter()).unwrap(), 0);
        // DISCARD renames: the earlier bind keeps its bytes, the new one knows only its own.
        assert!(!Arc::ptr_eq(&first, &ring.allocation));
        assert_eq!(first.lock().unwrap().known, [0..36, 48..84, 96..132]);
        let wrapped = ring.allocation.lock().unwrap();
        assert_eq!(wrapped.known, std::slice::from_ref(&(0..36)));
        assert_eq!(
            &wrapped.bytes[..36],
            &bytemuck::bytes_of(&instance(4.))[..36]
        );
        drop(wrapped);
        assert!(ring.append([instance(5.); 5].iter()).is_err());
    }

    #[test]
    fn type_grouping_preserves_actor_and_subsystem_order() {
        let mut emitter = super::super::tests::activation_test_emitter();
        let mut packets = Vec::new();
        for (registration, time) in [(2, 10.), (0, 20.), (2, 11.), (1, 30.), (0, 21.)] {
            emitter.registration = registration;
            packets.push(DrawPacket::new(&emitter, 1., Transform::IDENTITY, time));
        }
        let ordered = submission_order(&packets, 0);
        assert_eq!(
            ordered.iter().map(|p| p.constants.time).collect::<Vec<_>>(),
            [20., 21., 30., 10., 11.]
        );
    }

    #[test]
    fn ordinary_packets_precede_trails_within_each_actor_only() {
        let mut emitter = super::super::tests::activation_test_emitter();
        let mut packets = Vec::new();
        for (actor, trail, time) in [(0, true, 1.), (0, false, 2.), (1, true, 3.), (1, false, 4.)] {
            emitter.render.trail = trail;
            let mut packet = DrawPacket::new(&emitter, 1., Transform::IDENTITY, time);
            packet.actor = [0, actor, 0, 0];
            packets.push(packet);
        }
        assert_eq!(
            submission_order(&packets, 0)
                .iter()
                .map(|p| p.constants.time)
                .collect::<Vec<_>>(),
            [2., 1., 4., 3.]
        );
    }

    #[test]
    fn view_admission_filters_actors_before_registration_and_trail_order() {
        let mut emitter = super::super::tests::activation_test_emitter();
        let mut packets = Vec::new();
        let second = ViewMask::Some(vec![false, true]);
        let third = ViewMask::Some(vec![false, false, true]);
        for (reference, trail, views, time) in [
            (0, true, &second, 1.),
            (0, false, &second, 2.),
            (1, true, &third, 3.),
            (1, false, &third, 4.),
        ] {
            emitter.render.trail = trail;
            let mut packet = DrawPacket::new(&emitter, 1., Transform::IDENTITY, time);
            packet.actor = [0, 0, 0, reference];
            packet.views = views.clone();
            packets.push(packet);
        }
        assert!(submission_order(&packets, 0).is_empty());
        assert_eq!(
            submission_order(&packets, 1)
                .iter()
                .map(|p| p.constants.time)
                .collect::<Vec<_>>(),
            [2., 1.]
        );
        assert_eq!(
            submission_order(&packets, 2)
                .iter()
                .map(|p| p.constants.time)
                .collect::<Vec<_>>(),
            [4., 3.]
        );
        //Packet masks, identities and populations remain immutable across views.
        assert_eq!(
            packets.iter().map(|p| p.views.clone()).collect::<Vec<_>>(),
            [second.clone(), second, third.clone(), third]
        );
    }

    /// A free camera's frame can have more views than any fixed mask holds.
    #[test]
    fn view_masks_hold_any_number_of_views() {
        let mut views = vec![false; 12];
        views[9..].fill(true);
        let mask = ViewMask::Some(views);
        assert!(!mask.contains(8) && mask.contains(9) && mask.contains(11));
        assert!(!mask.contains(12) && !mask.is_empty());
        assert!(ViewMask::Some(vec![false; 3]).is_empty() && ViewMask::Some(Vec::new()).is_empty());
        assert!(ViewMask::All.contains(1_000) && !ViewMask::All.is_empty());
        let emitter = super::super::tests::activation_test_emitter();
        let mut packet = DrawPacket::new(&emitter, 1., Transform::IDENTITY, 1.);
        packet.views = mask;
        let packets = [packet];
        assert!(submission_order(&packets, 8).is_empty());
        assert_eq!(submission_order(&packets, 10).len(), 1);
    }
}
