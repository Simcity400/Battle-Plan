use shader_pipeline::gpu::stats::CountedPasses as _;
#[path = "native/packing.rs"]
mod packing;

use super::{Emitter, Simulation};
use eframe::wgpu;
#[cfg(test)]
use eframe::wgpu::util::DeviceExt;
use content_reader_mesh::{mesh::Transform, particles::RenderDescriptor};
use content_reader_gui::pdxshader::program::Stage;
use shader_pipeline::{binding, gpu, library::Effect};
#[cfg(test)]
use std::collections::BTreeSet;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};
use viewer_scene_base::{particle_shader::{self, Instance, Packet}, shader_scene::Partial};
use viewer_gpu::gpu_contract::{self, Targets};

pub struct Inputs<'a> {
    pub frame: &'a [u8],
    pub grid: &'a [u8],
    pub textures: [&'a wgpu::TextureView; 10],
    pub retained: &'a mut viewer_scene_base::shader_scene::State,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ViewMask {
    All,
    Some(Vec<bool>),
}

impl ViewMask {
    pub(super) fn contains(&self, view: usize) -> bool {
        match self {
            Self::All => true,
            Self::Some(views) => views.get(view).copied().unwrap_or(false),
        }
    }
}

pub(super) struct DrawPacket {
    page: usize,
    registration: usize,
    pub actor: [usize; 4],
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
        out.instances.reserve(sim.particles.len());
        for p in &sim.particles {
            let position = if e.local_space {
                p.position
            } else {
                std::array::from_fn(|i| p.position[i] + anchor[i] - sim.anchor[i])
            };
            let mut color = std::array::from_fn(|i| {
                e.color[i]
                    .animate_particle(p.color[i], p.age, p.life, sim.update_age)
                    .clamp(0., 255.)
                    .trunc() as u8
            });
            color[3] = (f32::from(color[3]) * alpha).round() as u8;
            out.instances.push(Instance {
                position_size: [
                    position[0],
                    position[1],
                    position[2],
                    e.size
                        .animate_particle(p.size, p.age, p.life, sim.update_age)
                        .max(0.),
                ],
                rotation: p.rotation,
                tile: p.tile,
                color,
                tail: [0; 12],
            });
        }
        out
    }

    pub fn entity(e: &Emitter, scale: f32, event: &viewer_models::entity_animation::Event) -> Self {
        Self::new(e, scale, event.transform, event.age - e.start.base)
    }
}

#[derive(Default)]
pub struct EntitySimulations(
    BTreeMap<String, BTreeMap<(viewer_models::entity_animation::EventKey, usize), EntityRuntime>>,
);

struct EntityRuntime {
    simulation: Simulation,
    seen: Option<usize>,
}

impl EntitySimulations {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn prepare(
        &mut self,
        entities: &BTreeMap<String, (f32, Vec<Emitter>)>,
        events: &[viewer_models::entity_animation::Event],
    ) -> io::Result<()> {
        let reset_span = viewer_runtime::profiling::span("entity_reset");
        for runtimes in self.0.values_mut() {
            for runtime in runtimes.values_mut() {
                runtime.seen = None;
            }
        }
        drop(reset_span);
        let register_span = viewer_runtime::profiling::span("entity_register");
        let mut failure = None;
        'events: for (at, event) in events.iter().enumerate() {
            let Some(name) = &event.definition.particle else {
                continue;
            };
            let Some((_, emitters)) = entities.get(name) else {
                continue;
            };
            if emitters.is_empty() {
                continue;
            }
            if !self.0.contains_key(name) {
                self.0.insert(name.clone(), BTreeMap::new());
            }
            #[expect(clippy::mutable_key_type, reason = "asset keys cache display data outside equality and ordering")]
            let runtimes = self.0.get_mut(name).expect("registered particle name");
            for (ordinal, emitter) in emitters.iter().enumerate() {
                let runtime = runtimes
                    .entry((event.key.clone(), ordinal))
                    .or_insert_with(|| EntityRuntime {
                        simulation: Simulation::cold(
                            emitter,
                            u64::from(event.seed) ^ ((ordinal as u64) << 32),
                            event.transform.translation,
                            event.age,
                        ),
                        seen: None,
                    });
                if runtime.seen.is_some() {
                    failure = Some(io::Error::other(format!(
                        "Map shader particles: 1 refused actor {name}: duplicate presentation identity {:?}",
                        event.key
                    )));
                    break 'events;
                }
                runtime.seen = Some(at);
            }
        }
        drop(register_span);
        let advance_span = viewer_runtime::profiling::span("entity_advance");
        {
            for (name, runtimes) in self.0.iter_mut() {
                let Some((scale, emitters)) = entities.get(name) else {
                    continue;
                };
                let mut seen = Vec::new();
                let mut members = Vec::new();
                for ((_, ordinal), runtime) in runtimes.iter_mut() {
                    let Some(at) = runtime.seen else {
                        continue;
                    };
                    let emitter = &emitters[*ordinal];
                    seen.push(at);
                    members.push(super::Member {
                        sim: &mut runtime.simulation,
                        emitter,
                        space: Some(super::motion::Space::new(
                            emitter.local_space,
                            emitter.system_local_space,
                            *scale,
                            events[at].transform,
                        )),
                    });
                }
                let mut start = 0;
                while start < members.len() {
                    let at = seen[start];
                    let end = start + seen[start..].iter().take_while(|&&other| other == at).count();
                    let event = &events[at];
                    super::advance_event_system(&mut members[start..end], event.age, event.emission_end);
                    start = end;
                }
            }
        }
        drop(advance_span);
        if viewer_runtime::view_record::on() {
            use viewer_runtime::view_record::add;
            let [mut runtimes, mut particles, mut advanced, mut advanced_particles] = [0; 4];
            for held in self.0.values() {
                runtimes += held.len() as u64;
                for runtime in held.values() {
                    let held = runtime.simulation.particles.len() as u64;
                    particles += held;
                    if runtime.seen.is_some() {
                        advanced += 1;
                        advanced_particles += held;
                    }
                }
            }
            add("entity effects", "events", events.len() as u64);
            add(
                "entity effects",
                "events_hidden",
                events.iter().filter(|event| event.hidden).count() as u64,
            );
            add("entity effects", "runtimes", runtimes);
            add("entity effects", "particles", particles);
            add("entity effects", "advanced", advanced);
            add("entity effects", "advanced_particles", advanced_particles);
        }
        if let Some(error) = failure {
            return Err(error);
        }
        let _retire_span = viewer_runtime::profiling::span("entity_retire");
        let alive = |runtime: &EntityRuntime| {
            runtime.seen.is_some()
                && (runtime.simulation.emitting || !runtime.simulation.particles.is_empty())
        };
        self.0.retain(|_, runtimes| {
            #[expect(clippy::mutable_key_type, reason = "asset keys cache display data outside equality and ordering")]
            let held: std::collections::BTreeSet<_> = runtimes
                .iter()
                .filter(|(_, runtime)| alive(runtime))
                .map(|((key, _), _)| key.clone())
                .collect();
            runtimes.retain(|(key, _), runtime| runtime.seen.is_some() && held.contains(key));
            !runtimes.is_empty()
        });
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn presentation_state(&self) -> String {
        self.0.iter().flat_map(|(name, runtimes)| {
            runtimes.iter().map(move |((key, ordinal), runtime)| {
                let sim = &runtime.simulation;
                format!(
                    "{name} {:?} {ordinal} age={} time={} seed={} emitting={} accumulator={} particles={:?}\n",
                    key.without_pass(), sim.age, sim.time, sim.seed, sim.emitting, sim.accumulator,
                    sim.particles.iter().map(|p| (p.age, p.life, p.position, p.size, p.rotation)).collect::<Vec<_>>()
                )
            })
        }).collect()
    }

    pub(super) fn sort(&mut self, eye: super::sort::Eye) {
        for runtime in self.0.values_mut().flat_map(BTreeMap::values_mut) {
            runtime.simulation.sort(eye);
        }
    }

    pub fn living(&self, event: &viewer_models::entity_animation::Event) -> bool {
        let Some(name) = &event.definition.particle else {
            return false;
        };
        self.0.get(name).is_some_and(|runtimes| {
            runtimes
                .range((event.key.clone(), 0)..=(event.key.clone(), usize::MAX))
                .any(|(_, runtime)| !runtime.simulation.particles.is_empty())
        })
    }

    pub(super) fn packets(
        &self,
        entities: &BTreeMap<String, (f32, Vec<Emitter>)>,
        events: &[viewer_models::entity_animation::Event],
        packets: &mut Vec<DrawPacket>,
    ) {
        let tasks: Vec<_> = events
            .iter()
            .enumerate()
            .filter_map(|(actor, event)| {
                if event.hidden {
                    return None;
                }
                let name = event.definition.particle.as_ref()?;
                let (scale, emitters) = entities.get(name)?;
                #[expect(clippy::mutable_key_type, reason = "asset keys cache display data outside equality and ordering")]
                let runtimes = self.0.get(name)?;
                Some((actor, event, *scale, emitters, runtimes))
            })
            .collect();
        for (actor, event, scale, emitters, runtimes) in tasks {
            for (ordinal, emitter) in emitters.iter().enumerate() {
                let Some(runtime) = runtimes.get(&(event.key.clone(), ordinal)) else {
                    continue;
                };
                let sim = &runtime.simulation;
                let mut packet = DrawPacket::entity(emitter, scale, event);
                packet.actor = [1, actor, 0, 0];
                packet.constants.time = sim.age;
                packet.instances.reserve(sim.particles.len());
                for particle in &sim.particles {
                    let position = particle.position;
                    packet.instances.push(Instance {
                        position_size: [
                            position[0],
                            position[1],
                            position[2],
                            emitter
                                .size
                                .animate_particle(
                                    particle.size,
                                    particle.age,
                                    particle.life,
                                    sim.update_age,
                                )
                                .max(0.),
                        ],
                        rotation: particle.rotation,
                        tile: particle.tile,
                        color: std::array::from_fn(|i| {
                            emitter.color[i]
                                .animate_particle(
                                    particle.color[i],
                                    particle.age,
                                    particle.life,
                                    sim.update_age,
                                )
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

fn order_key(packet: &DrawPacket) -> (usize, [usize; 4], bool) {
    (packet.registration, packet.actor, packet.descriptor.trail)
}

pub(super) fn order(packets: &mut [DrawPacket]) {
    packets.sort_by_key(order_key);
}

#[cfg(test)]
fn submission_order(packets: &[DrawPacket], view: usize) -> Vec<&DrawPacket> {
    let mut ordered: Vec<_> = packets
        .iter()
        .filter(|packet| packet.views.contains(view))
        .collect();
    ordered.sort_by_key(|packet| order_key(packet));
    ordered
}

const TRAIL_STRIDE: usize = 56 * 28;

pub(super) const MAX_PARTICLES: usize = i32::MAX as usize / TRAIL_STRIDE;

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

    fn append<'a>(
        &mut self,
        instances: impl Iterator<Item = &'a Instance> + Clone,
    ) -> io::Result<u64> {
        self.append_counted(instances.clone(), instances.count())
    }

    fn append_counted<'a>(
        &mut self,
        instances: impl Iterator<Item = &'a Instance>,
        count: usize,
    ) -> io::Result<u64> {
        let length = (count as u64)
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

struct DrawConstants {
    scale: [f32; 4],
    object: [u8; 96],
}

impl DrawConstants {
    fn new(packet: &Packet) -> Self {
        let mut object = [0; 96];
        object[..64].copy_from_slice(bytemuck::bytes_of(&packet.world));
        let atlas = [
            0_f32,
            0.,
            packet.tiles[0] as f32,
            packet.tiles[1] as f32,
            packet.time,
        ];
        object[64..84].copy_from_slice(bytemuck::bytes_of(&atlas));
        Self {
            scale: [packet.scale[0], packet.scale[1], 0., 0.],
            object,
        }
    }
}

fn same_packet_bindings(a: &DrawPacket, b: &DrawPacket) -> bool {
    a.page == b.page
        && bytemuck::bytes_of(&a.constants.world) == bytemuck::bytes_of(&b.constants.world)
        && bytemuck::bytes_of(&a.constants.scale) == bytemuck::bytes_of(&b.constants.scale)
        && a.constants.tiles == b.constants.tiles
        && a.constants.time.to_bits() == b.constants.time.to_bits()
}

struct QuadUpload {
    native: gpu::VertexLayout,
    layout: gpu::VertexLayout,
    vertices: gpu::Slice,
    #[cfg(test)]
    bytes: Vec<u8>,
}

struct ActorUpload {
    native: Vec<gpu::VertexLayout>,
    layouts: [gpu::VertexLayout; 2],
    vertices: [gpu::Slice; 2],
    #[cfg(test)]
    bytes: [Vec<u8>; 2],
}

struct PacketUpload<'a> {
    layouts: &'a [gpu::VertexLayout],
    vertices: [gpu::Slice; 2],
    #[cfg(test)]
    bytes: [Vec<u8>; 2],
}

#[derive(Default)]
struct OrdinaryUploads {
    quads: Vec<QuadUpload>,
    actors: Vec<ActorUpload>,
    start: u64,
    count: u64,
}

impl OrdinaryUploads {
    fn begin(&mut self, start: u64, end: u64) {
        self.actors.clear();
        self.start = start;
        self.count = (end - start) / 48;
    }

    fn packet<'a>(
        &'a mut self,
        arena: &gpu::Arena,
        layouts: &[gpu::VertexLayout],
        ring: &InstanceRing,
        quad: &[u8],
        offset: u64,
        count: u32,
    ) -> Option<PacketUpload<'a>> {
        use shader_pipeline::vertices::{NativeBuffer, pack_native};
        if layouts.len() != 2
            || layouts[0].stride != 8
            || layouts[0].step != wgpu::VertexStepMode::Vertex
            || layouts[1].stride != 48
            || layouts[1].step != wgpu::VertexStepMode::Instance
        {
            return None;
        }
        let actor_count = u32::try_from(self.count).ok()?;
        let at = if let Some(at) = self.actors.iter().position(|p| p.native == layouts) {
            at
        } else {
            let q = if let Some(at) = self.quads.iter().position(|p| p.native == layouts[0]) {
                at
            } else {
                let known = 0..quad.len() as u64;
                let buffers = BTreeMap::from([(
                    0,
                    Some(NativeBuffer {
                        bytes: quad,
                        size: quad.len() as u64,
                        offset: 0,
                        known: std::slice::from_ref(&known),
                    }),
                )]);
                let mut packed = pack_native(&layouts[..1], &buffers, 4, 1).ok()?;
                let bytes = packed.bytes.pop()?;
                let vertices = arena.vertices(&bytes).ok()?;
                self.quads.push(QuadUpload {
                    native: layouts[0].clone(),
                    layout: packed.layouts.pop()?,
                    vertices,
                    #[cfg(test)]
                    bytes,
                });
                self.quads.len() - 1
            };
            let allocation = ring.allocation.lock().unwrap_or_else(|e| e.into_inner());
            let buffers = BTreeMap::from([(
                0,
                Some(NativeBuffer {
                    bytes: &allocation.bytes,
                    size: allocation.size,
                    offset: self.start,
                    known: &allocation.known,
                }),
            )]);
            let mut packed = pack_native(&layouts[1..], &buffers, 4, actor_count).ok()?;
            let bytes = packed.bytes.pop()?;
            let vertices = arena.vertices(&bytes).ok()?;
            let quad = &self.quads[q];
            self.actors.push(ActorUpload {
                native: layouts.to_vec(),
                layouts: [quad.layout.clone(), packed.layouts.pop()?],
                vertices: [quad.vertices.clone(), vertices],
                #[cfg(test)]
                bytes: [quad.bytes.clone(), bytes],
            });
            self.actors.len() - 1
        };
        let upload = &self.actors[at];
        let first = (offset - self.start) / 48;
        let stride = upload.layouts[1].stride;
        let byte_offset = first.checked_mul(stride)?;
        let byte_length = u64::from(count).checked_mul(stride)?;
        Some(PacketUpload {
            layouts: &upload.layouts,
            vertices: [
                upload.vertices[0].clone(),
                gpu::Slice {
                    buffer: upload.vertices[1].buffer.clone(),
                    offset: upload.vertices[1].offset + byte_offset,
                    size: byte_length.max(4),
                },
            ],
            #[cfg(test)]
            bytes: [
                upload.bytes[0].clone(),
                upload.bytes[1][byte_offset as usize..(byte_offset + byte_length) as usize]
                    .to_vec(),
            ],
        })
    }
}

#[derive(Default)]
struct Applied {
    scale: Option<[u8; 16]>,
    object: Option<[u8; 96]>,
    quad: bool,
    program: Option<usize>,
    texture: Option<(usize, usize)>,
}

impl Applied {
    fn scale(&mut self, state: &mut viewer_scene_base::shader_scene::State, bytes: &[u8]) -> bool {
        if self
            .scale
            .as_ref()
            .is_some_and(|bound| bound.as_slice() == bytes)
        {
            return true;
        }
        for stage in [Stage::Vertex, Stage::Pixel] {
            state.stream.bind_partial_constants(
                2,
                stage,
                Some(bytes),
                std::slice::from_ref(&(0..8)),
            );
        }
        self.scale = Some(bytes.try_into().expect("particle scale allocation"));
        false
    }

    fn object(&mut self, state: &mut viewer_scene_base::shader_scene::State, bytes: &[u8]) -> bool {
        if self
            .object
            .as_ref()
            .is_some_and(|bound| bound.as_slice() == bytes)
        {
            return true;
        }
        state.stream.bind_partial_constants(
            3,
            Stage::Vertex,
            Some(bytes),
            std::slice::from_ref(&(0..84)),
        );
        self.object = Some(bytes.try_into().expect("particle object allocation"));
        false
    }

    fn quad(&mut self, state: &mut viewer_scene_base::shader_scene::State, bytes: &Arc<[u8]>) -> bool {
        if self.quad {
            return true;
        }
        state.bind_vertex(0, 8, bytes.clone());
        self.quad = true;
        false
    }
}

#[derive(Default)]
struct Techniques {
    names: BTreeMap<String, [Option<usize>; 4]>,
    programs: Vec<NamedProgram>,
}

struct NamedProgram {
    name: String,
    trail: bool,
    program: Option<Program>,
    cache: shader_pipeline::stream::NativePassCache,
}

impl Techniques {
    fn get(&mut self, effect: &Arc<Effect>, descriptor: &RenderDescriptor) -> io::Result<usize> {
        let flags = usize::from(descriptor.trail) * 2 + usize::from(descriptor.billboard);
        if let Some(index) = self
            .names
            .get(&descriptor.shader)
            .and_then(|names| names[flags])
        {
            return Ok(index);
        }
        let name = descriptor.technique();
        let index = if let Some(index) = self
            .programs
            .iter()
            .position(|p| p.name == name && p.trail == descriptor.trail)
        {
            index
        } else {
            let program = Program::new(effect, &name, descriptor.trail)?;
            self.programs.push(NamedProgram {
                name,
                trail: descriptor.trail,
                program,
                cache: Default::default(),
            });
            self.programs.len() - 1
        };
        self.names.entry(descriptor.shader.clone()).or_default()[flags] = Some(index);
        Ok(index)
    }
}

struct PreviousDraw {
    packet: usize,
    cached: usize,
    trail: bool,
    pass: shader_pipeline::stream::PassSubmission,
}

#[cfg(test)]
struct DrawTrace {
    packet: usize,
    planned: bool,
    prepared: bool,
    reused: bool,
    bytes: Vec<Vec<u8>>,
    layouts: Vec<gpu::VertexLayout>,
    bindings: gpu::Bindings,
    retained: viewer_scene_base::shader_scene::State,
}

pub(super) struct Renderer {
    effect: Arc<Effect>,
    pages: Vec<wgpu::TextureView>,
    samples: u32,
    indices: wgpu::Buffer,
    quad: Arc<[u8]>,
    retained: viewer_scene_base::shader_scene::State,
    max_particles: usize,
    techniques: Techniques,
    packing: packing::Uploads,
    #[cfg(test)]
    trace: Option<Vec<DrawTrace>>,
}

struct Program {
    packing: Option<Arc<packing::Plan>>,
    selection: shader_pipeline::library::Selection,
    layouts: Vec<gpu::VertexLayout>,
    state: gpu::State,
    raster: content_reader_gui::pdxshader::RasterizerState,
}

impl Program {
    fn new(effect: &Arc<Effect>, name: &str, trail: bool) -> io::Result<Option<Self>> {
        shader_pipeline::library::with_recorded(effect, |effect| {
            let Some(selection) = effect.select(name).map_err(io::Error::other)? else {
                return Ok(None);
            };
            let compiled = effect.compiled(selection.index).map_err(io::Error::other)?;
            let technique = &effect.source.techniques[selection.index];
            let layouts = particle_shader::layouts(&compiled.vertex.inputs, trail)?;
            let packing = (!trail).then(|| packing::Plan::new(&layouts)).flatten();
            Ok(Some(Self {
                packing,
                layouts,
                state: gpu::State::authored(&effect.source, technique).map_err(io::Error::other)?,
                raster: effect.source.rasterizer(technique),
                selection,
            }))
        })
    }
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
            shader_pipeline::gpu::stats::create_buffer_init(
                &device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                },
            )
        };
        Self {
            effect,
            pages,
            samples,
            max_particles,
            retained: Default::default(),
            techniques: Techniques::default(),
            packing: packing::Uploads::default(),
            #[cfg(test)]
            trace: None,
            quad: bytemuck::cast_slice(&particle_shader::QUAD).into(),
            indices: buffer(
                "native particle indices",
                bytemuck::cast_slice(&particle_shader::INDICES),
                wgpu::BufferUsages::INDEX,
            ),
        }
    }

    pub(super) fn warm(&mut self, device: &wgpu::Device, emitters: &[&Emitter]) {
        let mut stream = shader_pipeline::stream::Stream::default();
        stream.cache = std::mem::take(&mut self.retained.stream.cache);
        let mut seen = std::collections::BTreeSet::new();
        let (mut prepared, mut recorded, mut refused, mut retained) = (0, 0, 0, 0);
        for emitter in emitters {
            let name = emitter.render.technique();
            if !seen.insert((name.clone(), emitter.render.trail)) {
                continue;
            }
            let result = (|| -> io::Result<Option<shader_pipeline::stream::WarmPass>> {
                let index = self.techniques.get(&self.effect, &emitter.render)?;
                let Some(program) = &self.techniques.programs[index].program else {
                    return Ok(None);
                };
                stream.begin_frame();
                stream.select_program(&program.selection, &program.layouts);
                stream.bind_blend(program.state.blend, program.state.write_mask);
                stream.bind_rasterizer(program.raster.clone());
                for (key, sampler) in viewer_scene_base::mesh_shader::map_samplers() {
                    if key % 2 == 1 && (4..14).contains(&(key / 2)) {
                        stream.bind_texture(key / 2, Stage::Pixel, None, Some(&sampler));
                    }
                }
                stream.bind_texture(
                    0,
                    Stage::Pixel,
                    None,
                    program.state.samplers.get(&binding(0, Stage::Pixel)),
                );
                let buffers = (0..program.layouts.len())
                    .map(|slot| (slot as u32, None))
                    .collect();
                let layouts =
                    shader_pipeline::vertices::pack_native(&program.layouts, &buffers, 0, 0)
                        .map_err(io::Error::other)?
                        .layouts;
                let topology = if emitter.render.trail {
                    wgpu::PrimitiveTopology::TriangleStrip
                } else {
                    wgpu::PrimitiveTopology::TriangleList
                };
                stream
                    .warm_pass(device, &particle_pass(self.samples, topology), layouts)
                    .map(Some)
                    .map_err(io::Error::other)
            })();
            match result {
                Ok(Some(shader_pipeline::stream::WarmPass::Selected)) => prepared += 1,
                Ok(Some(shader_pipeline::stream::WarmPass::Recorded(why))) => {
                    prepared += 1;
                    recorded += 1;
                    eprintln!(
                        "particle warm-up {name:?}: recorded pipeline prepared after replacement refusal: {why}"
                    );
                }
                Ok(None) => {
                    retained += 1;
                    eprintln!(
                        "particle warm-up {name:?}: missing technique retains runtime program"
                    );
                }
                Err(why) => {
                    refused += 1;
                    eprintln!("particle warm-up {name:?} refused: {why}");
                }
            }
        }
        self.retained.stream.cache = stream.cache;
        eprintln!(
            "particle warm-up: {prepared} prepared ({recorded} recorded), {retained} retained, {refused} refused"
        );
    }

    #[expect(clippy::too_many_arguments, reason = "the arena, encoder and targets, plus the projection, eye, view, inputs and packets of one draw")]
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
        let _profile = viewer_runtime::profiling::thread_span("weather_prepare_lower");
        let mut planned_uploads = std::mem::take(&mut self.packing);
        planned_uploads.clear_view();
        let mut applied = Applied::default();
        self.retained.inherit(inputs.retained);
        let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compiled particles"),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            ..Default::default()
        });
        let ordered: Vec<_> = packets
            .iter()
            .filter(|packet| packet.views.contains(view))
            .collect();
        if viewer_runtime::view_record::on() {
            use viewer_runtime::view_record::add;
            let [mut empty, mut instances, mut trails] = [0; 3];
            for packet in &ordered {
                empty += u64::from(packet.instances.is_empty());
                instances += packet.instances.len() as u64;
                trails += u64::from(packet.descriptor.trail);
            }
            add("weather draw", "views", 1);
            add("weather draw", "packets", ordered.len() as u64);
            add("weather draw", "packets_empty", empty);
            add("weather draw", "instances", instances);
            add("weather draw", "trails", trails);
        }
        let mut actor = None;
        let mut ordinary_offset = 0;
        let mut ring = InstanceRing::new(allocation_size(self.max_particles, 48)?);
        let trail_size = allocation_size(self.max_particles, TRAIL_STRIDE)?;
        let depth = particle_depth();
        let [list, strip] = [
            particle_pass(self.samples, wgpu::PrimitiveTopology::TriangleList),
            particle_pass(self.samples, wgpu::PrimitiveTopology::TriangleStrip),
        ];
        {
            let stream = &mut self.retained.stream;
            for stage in [Stage::Vertex, Stage::Pixel] {
                stream.bind_partial_constants(0, stage, Some(inputs.frame), &[0..348, 352..408]);
                stream.bind_constants(1, stage, Some(bytemuck::bytes_of(&projection)));
            }
            stream.bind_constants(2, Stage::Pixel, Some(inputs.grid));
            stream.bind_depth_stencil(depth);
            let map_samplers = viewer_scene_base::mesh_shader::map_samplers();
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
        let quad = &self.quad;
        let mut previous: Option<PreviousDraw> = None;
        let mut bound: Vec<Option<gpu::Slice>> = Vec::new();
        let mut drawn = 0u64;
        let mut uploads = OrdinaryUploads::default();
        shader_pipeline::gpu::stats::state();
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        for (index, packet) in ordered.iter().enumerate() {
            let key_actor = (packet.registration, packet.actor);
            if actor != Some(key_actor) {
                actor = Some(key_actor);
                let scale = [packet.constants.scale[0], packet.constants.scale[1], 0., 0.];
                applied.scale(&mut self.retained, bytemuck::bytes_of(&scale));
                applied.quad(&mut self.retained, quad);
                let instances = ordered[index..]
                    .iter()
                    .take_while(|p| (p.registration, p.actor) == key_actor && !p.descriptor.trail)
                    .flat_map(|p| &p.instances);
                ordinary_offset = ring.append(instances)?;
                uploads.begin(ordinary_offset, ring.cursor);
                planned_uploads.begin(ordinary_offset, ring.cursor);
            }
            if packet.instances.is_empty() {
                continue;
            }
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
            drawn += 1;
            let cached = self.techniques.get(&self.effect, &packet.descriptor)?;
            let named = &mut self.techniques.programs[cached];
            let (name, program, native_cache) = (
                named.name.as_str(),
                &named.program,
                named.program.as_ref().map(|_| &mut named.cache),
            );
            let constants = DrawConstants::new(&packet.constants);
            let scale = bytemuck::bytes_of(&constants.scale);
            let object = constants.object.as_slice();
            if applied.program != Some(cached) {
                let stream = &mut self.retained.stream;
                match program.as_ref() {
                    Some(program) => {
                        stream.select_program(&program.selection, &program.layouts);
                        stream.bind_blend(program.state.blend, program.state.write_mask);
                        stream.bind_rasterizer(program.raster.clone());
                    }
                    None => {
                        stream.bind_program(None, Vec::new());
                        stream.bind_blend(None, wgpu::ColorWrites::ALL);
                        stream.bind_rasterizer(Default::default());
                    }
                }
                applied.program = Some(cached);
            }
            applied.scale(&mut self.retained, scale);
            applied.object(&mut self.retained, object);
            let texture = Some((packet.page, cached));
            if applied.texture != texture {
                self.retained.stream.bind_texture(
                    0,
                    Stage::Pixel,
                    Some(self.pages[packet.page].clone()),
                    program
                        .as_ref()
                        .and_then(|program| program.state.samplers.get(&binding(0, Stage::Pixel))),
                );
                applied.texture = texture;
            }
            let packet_offset = ordinary_offset;
            if let Some(vertices) = &trail {
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
                applied.quad = false;
            } else {
                applied.quad(&mut self.retained, quad);
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
            #[cfg(test)]
            let mut planned = false;
            let upload = if trail.is_none() {
                program.as_ref().and_then(|program| {
                    let attempt =
                        program
                            .packing
                            .as_ref()
                            .map_or(packing::Attempt::Unproved, |plan| {
                                planned_uploads.packet(
                                    arena,
                                    plan,
                                    &ring,
                                    quad,
                                    packet_offset,
                                    instances,
                                )
                            });
                    match attempt {
                        packing::Attempt::Handled(upload) => {
                            #[cfg(test)]
                            {
                                planned = upload.is_some();
                            }
                            upload
                        }
                        packing::Attempt::Unproved => uploads.packet(
                            arena,
                            &program.layouts,
                            &ring,
                            quad,
                            packet_offset,
                            instances,
                        ),
                    }
                })
            } else {
                None
            };
            let preceding = previous
                .as_ref()
                .filter(|last| {
                    program.is_some()
                        && last.cached == cached
                        && last.trail == trail.is_some()
                        && same_packet_bindings(packet, ordered[last.packet])
                })
                .map(|last| &last.pass);
            let template = if trail.is_some() { &strip } else { &list };
            let submission = self.retained.prepare_with_cache(
                arena,
                format_args!("particle {name}"),
                template,
                count,
                instances,
                upload
                    .as_ref()
                    .map(|p| shader_pipeline::stream::NativePrepared {
                        layouts: p.layouts,
                        vertices: &p.vertices,
                    }),
                preceding,
                native_cache,
            )?;
            #[cfg(test)]
            if let Some(trace) = &mut self.trace {
                let (layouts, bytes) = if let Some(upload) = &upload {
                    (upload.layouts.to_vec(), upload.bytes.to_vec())
                } else {
                    let packed = self
                        .retained
                        .packed_bytes(count, instances)
                        .expect("a prepared native draw can be packed again");
                    (packed.layouts, packed.bytes)
                };
                let retained = self.retained.snapshot();
                trace.push(DrawTrace {
                    packet: index,
                    planned,
                    prepared: upload.is_some(),
                    reused: preceding.is_some(),
                    bytes,
                    layouts,
                    bindings: submission.pass.bindings.clone(),
                    retained,
                });
            }
            let pipeline = &submission.pass.pipeline;
            let same = previous
                .as_ref()
                .filter(|last| Arc::ptr_eq(&last.pass.pipeline, pipeline))
                .map(|last| &last.pass.bindings);
            if same.is_none() {
                shader_pipeline::gpu::stats::pipeline();
                pass.set_pipeline(&pipeline.pipeline);
                pass.set_stencil_reference(submission.pass.stencil_reference.unwrap_or(0));
                shader_pipeline::gpu::stats::state();
                pass.set_blend_constant(wgpu::Color::WHITE);
            }
            submission.pass.bindings.bind_after(&mut pass, same);
            for (slot, buffer) in submission.vertices.iter().enumerate() {
                let held = bound.get(slot).and_then(Option::as_ref);
                if held.is_some_and(|held: &gpu::Slice| {
                    held.buffer == buffer.buffer
                        && held.offset == buffer.offset
                        && held.size == buffer.size
                }) {
                    continue;
                }
                shader_pipeline::gpu::stats::state();
                pass.set_vertex_buffer(slot as u32, buffer.slice());
                if bound.len() <= slot {
                    bound.resize(slot + 1, None);
                }
                bound[slot] = Some(buffer.clone());
            }
            if trail.is_some() {
                shader_pipeline::gpu::stats::draw();
                pass.draw(0..count, 0..instances);
            } else {
                shader_pipeline::gpu::stats::draw();
                pass.draw_indexed(0..6, 0, 0..instances);
            }
            previous = Some(PreviousDraw {
                packet: index,
                cached,
                trail: trail.is_some(),
                pass: submission.pass,
            });
        }
        inputs.retained.inherit(&self.retained);
        planned_uploads.clear_view();
        self.packing = planned_uploads;
        viewer_runtime::view_record::add("weather draw", "packets_drawn", drawn);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_packets_equal(a: &[DrawPacket], b: &[DrawPacket]) {
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert_eq!(
                (a.page, a.registration, a.actor, &a.views),
                (b.page, b.registration, b.actor, &b.views)
            );
            assert_eq!(a.descriptor, b.descriptor);
            let a_constants = DrawConstants::new(&a.constants);
            let b_constants = DrawConstants::new(&b.constants);
            assert_eq!(
                bytemuck::bytes_of(&a_constants.scale),
                bytemuck::bytes_of(&b_constants.scale)
            );
            assert_eq!(a_constants.object, b_constants.object);
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&a.instances),
                bytemuck::cast_slice::<_, u8>(&b.instances)
            );
        }
    }

    #[test]
    fn bake_order_independent_events_keep_particle_packets_and_duplicate_refusals() {
        use viewer_models::order_fixture;
        let emitters = (0..3)
            .map(|at| {
                let mut emitter = super::super::test_support::activation_test_emitter();
                emitter.registration = [2, 0, 2][at];
                emitter.render.trail = at == 1;
                emitter
            })
            .collect();
        let entities = BTreeMap::from([("smoke".into(), (1., emitters))]);
        let mut reference: Option<Vec<Vec<DrawPacket>>> = None;
        let mut refusal = None;
        for retained in [false, true] {
            for order in order_fixture::ORDERS {
                let mut events = order_fixture::events(order, retained);
                let mut simulation = EntitySimulations::default();
                let mut frames = Vec::new();
                for age in [0., 0.13, 0.4, 0.8] {
                    for event in &mut events {
                        event.age = age;
                    }
                    simulation.prepare(&entities, &events).unwrap();
                    let mut packets = Vec::new();
                    simulation.packets(&entities, &events, &mut packets);
                    super::order(&mut packets);
                    frames.push(packets);
                }
                assert!(
                    frames
                        .last()
                        .unwrap()
                        .iter()
                        .any(|packet| !packet.instances.is_empty())
                );
                if let Some(reference) = &reference {
                    for (actual, expected) in frames.iter().zip(reference) {
                        assert_packets_equal(actual, expected);
                    }
                } else {
                    reference = Some(frames);
                }
                events.push(events[0].clone());
                let error = simulation
                    .prepare(&entities, &events)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains("duplicate presentation identity"));
                if let Some(expected) = &refusal {
                    assert_eq!(&error, expected);
                } else {
                    refusal = Some(error);
                }
            }
        }
    }

    #[test]
    fn entity_ticks_and_packets_preserve_runtime_and_submission_order() {
        use viewer_models::{self as models, entity_animation};
        let rig = entity_animation::pose_rig(1., false, true);
        let mut player = entity_animation::Player::new(&rig);
        let mut events = Vec::new();
        player.evaluate(
            &rig,
            models::ModelInstance {
                scale: 1.,
                animation_seed: 7,
                ..Default::default()
            },
            1.,
            &mut [],
            &mut events,
        );
        let prototype = events.remove(0);
        let entities: BTreeMap<_, _> = ["zeta", "alpha"]
            .into_iter()
            .map(|name| {
                (
                    name.to_owned(),
                    (
                        0.7,
                        (0..3)
                            .map(|ordinal| {
                                let mut e = super::super::test_support::activation_test_emitter();
                                e.registration = [2, 0, 2][ordinal];
                                e.render.trail = ordinal == 1;
                                e.local_space = ordinal == 2;
                                e.system_local_space = ordinal != 0;
                                e
                            })
                            .collect(),
                    ),
                )
            })
            .collect();
        let mut simulations = EntitySimulations::default();
        for (frame, age) in [0., 0.13, 0.4, 0.4, 1.3, 4., 0.2, 0.5]
            .into_iter()
            .enumerate()
        {
            let mut events: Vec<_> = (0..40)
                .filter(|actor| frame < 4 || actor % 3 != 0)
                .map(|actor| {
                    let mut event = prototype.clone();
                    event.key.model =
                        viewer_models::AssetKey::synthetic(&format!("actor_{}", 40 - actor));
                    event.seed = actor as u32;
                    event.definition.particle =
                        Some(if actor % 2 == 0 { "zeta" } else { "alpha" }.into());
                    event.age = age;
                    event.transform.translation = [actor as f32 + age, 1., -age];
                    event.hidden = actor % 7 == 0;
                    event.emission_end = (actor % 5 == 0).then_some(0.3);
                    event
                })
                .collect();
            if frame == 3 {
                events.push(events[2].clone());
            }
            let prepared = simulations
                .prepare(&entities, &events)
                .map_err(|error| error.to_string());
            if prepared.is_err() {
                assert_eq!(frame, 3, "only the duplicated event refuses");
                continue;
            }
            let mut a = Vec::new();
            simulations.packets(&entities, &events, &mut a);
            let expected: Vec<_> = events
                .iter()
                .enumerate()
                .filter(|(_, event)| !event.hidden)
                .flat_map(|(actor, event)| {
                    #[expect(clippy::mutable_key_type, reason = "asset keys cache display data outside equality and ordering")]
                    let runtimes = &simulations.0[event.definition.particle.as_ref().unwrap()];
                    (0..3).filter_map(move |ordinal| {
                        runtimes
                            .contains_key(&(event.key.clone(), ordinal))
                            .then_some(([1, actor, 0, 0], [2, 0, 2][ordinal]))
                    })
                })
                .collect();
            assert_eq!(
                a.iter()
                    .map(|packet| (packet.actor, packet.registration))
                    .collect::<Vec<_>>(),
                expected
            );
        }
        simulations.prepare(&BTreeMap::new(), &[]).unwrap();
        assert!(simulations.0.is_empty());
    }

    #[test]
    fn a_depth_sorted_runtime_submits_its_storage_after_one_pass_and_an_unsorted_one_as_held() {
        use viewer_models::{self as models, entity_animation};
        use content_reader_mesh::particles::Sort;
        use viewer_gpu::camera::{Camera, MapDefines, MapSize, Pose};
        let rig = entity_animation::pose_rig(1., false, true);
        let mut player = entity_animation::Player::new(&rig);
        let mut events = Vec::new();
        player.evaluate(
            &rig,
            models::ModelInstance {
                scale: 1.,
                animation_seed: 7,
                ..Default::default()
            },
            1.,
            &mut [],
            &mut events,
        );
        events.truncate(1);
        events[0].definition.particle = Some("storm".into());
        events[0].hidden = false;
        let emitters = [Sort::Depth, Sort::None]
            .map(|sort| {
                let mut e = super::super::test_support::activation_test_emitter();
                e.render.sort = sort;
                e
            })
            .into();
        let entities = BTreeMap::from([("storm".to_owned(), (1., emitters))]);
        let mut simulations = EntitySimulations::default();
        simulations.prepare(&entities, &events).unwrap();
        let heights = [50., 40., 30., 20., 10.];
        for runtime in simulations.0.get_mut("storm").unwrap().values_mut() {
            runtime.simulation.particles = heights
                .map(|y| super::super::LiveParticle {
                    age: 0.,
                    life: 10.,
                    mass: 1.,
                    position: [0., y, 0.],
                    velocity: [0.; 3],
                    size: 1.,
                    rotation: [0.; 3],
                    rotation_base: [0.; 3],
                    spin: [0.; 3],
                    color: [255.; 4],
                    tile: [0; 4],
                })
                .into();
        }
        let eye = super::super::sort::Eye::of(Camera {
            center: [0., 0.],
            height: 100.,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: Pose::Map,
        });
        for sorted in [heights.to_vec(), vec![40., 20., 50., 30., 10.]] {
            simulations.sort(eye);
            let mut packets = Vec::new();
            simulations.packets(&entities, &events, &mut packets);
            assert_eq!(
                packets
                    .iter()
                    .map(|packet| packet
                        .instances
                        .iter()
                        .map(|instance| instance.position_size[1])
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
                [sorted, heights.to_vec()]
            );
        }
    }

    #[test]
    fn weather_packets_match_source_order_reference_across_admission_changes() {
        use super::super::{
            Anchor,
            Assets,
            Presentation,
            Simulation,
            Simulations,
            WeatherPair,
            gates,
            motion,
            weather_packets,
        };
        use map_display_data::WeatherField as Field;
        use viewer_gpu::{camera::{Camera, MapDefines, MapSize, Pose}, province_admission::OPEN};
        use viewer_runtime::presentation::Continuity;
        let emitters = || {
            (0..3)
                .map(|ordinal| {
                    let mut e = super::super::test_support::activation_test_emitter();
                    e.registration = [2, 0, 2][ordinal];
                    e.actor = ordinal;
                    e.render.trail = ordinal == 1;
                    e.local_space = ordinal == 2;
                    e
                })
                .collect()
        };
        let assets = Assets {
            effect: Effect::from_compiled(Arc::new(Default::default()), Vec::new(), Vec::new()),
            max_particles: 100,
            anchors_by_region: BTreeMap::from([(7, (0..20).collect())]),
            region_provinces: Default::default(),
            graphics: super::super::tests::reference_defines(),
            anchors: (0..20)
                .map(|index| Anchor {
                    region: 7,
                    position: [index as f32 * 5., 10., 30.],
                    small: index % 2 == 0,
                })
                .collect(),
            systems: BTreeMap::from([(
                (0, false),
                vec![WeatherPair {
                    members: [emitters(), emitters()],
                    radii: [Some(15.); 2],
                    sounds: [Vec::new(), Vec::new()],
                }],
            )]),
            pages: Vec::new(),
            entities: BTreeMap::new(),
            withheld: Vec::new(),
            lights: Default::default(),
        };
        let field = Field {
            branch: 1,
            hour: 20,
            kinds: BTreeMap::from([(7, 0)]).into(),
        };
        let mut presentation = Presentation::default();
        presentation.transition(&Field::default(), &field, Continuity::Reset);
        let camera = Camera {
            center: [40., 30.],
            height: 300.,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: Pose::Map,
        };
        for held in [Presentation::default(), presentation.clone()] {
            let views = super::super::parent_views(
                &super::super::province_admission_views(camera, [1280, 720]),
                [1280, 720],
            );
            let expected: BTreeMap<_, _> =
                super::super::visible_emitters_at(&assets, &presentation, camera, &views, 0..assets.anchors.len())
                    .into_iter()
                    .filter(|visible| !held.0.contains_key(&assets.anchors[visible.key.0].region))
                    .map(|visible| {
                        let mut simulation =
                            Simulation::cold(visible.emitter, visible.seed, visible.anchor, 0.0);
                        simulation.prepared = true;
                        (visible.key, simulation)
                    })
                    .collect();
            let actual = super::super::prepare_changed(
                &assets,
                &presentation,
                &held,
                camera,
                [1280, 720],
            );
            assert_eq!(
                actual.keys().collect::<Vec<_>>(),
                expected.keys().collect::<Vec<_>>()
            );
            for (key, value) in actual {
                let reference = &expected[&key];
                assert_eq!(value.seed, reference.seed);
                assert_eq!(value.age.to_bits(), reference.age.to_bits());
                assert_eq!(value.anchor, reference.anchor);
                assert_eq!(value.prepared, reference.prepared);
                assert!(value.particles.is_empty());
            }
        }
        let (mut serial, mut made) = (Simulations::new(), Simulations::new());
        for (frame, time) in [0., 0.13, 0.4, 0.4, 1.3, 4., 0.2, 0.5]
            .into_iter()
            .enumerate()
        {
            let camera = Camera {
                center: [10., 20.],
                height: [300., 800., 1200.][frame % 3],
                map: MapSize::TEST,
                defines: MapDefines::test(),
                pose: Pose::Map,
            };
            let views = [
                (
                    [0.; 3],
                    [if frame == 4 { [0., 0., 0., 100.] } else { OPEN }; 6],
                ),
                ([0.; 3], [OPEN; 6]),
            ];
            let seen = super::super::visible_emitters_at(&assets, &presentation, camera, &views, 0..assets.anchors.len());
            let visible: BTreeSet<_> = seen.iter().map(|v| v.key).collect();
            let mut expected = Vec::new();
            for v in seen {
                let sim = serial
                    .entry(v.key)
                    .or_insert_with(|| Simulation::cold(v.emitter, v.seed, v.anchor, time));
                let e = v.emitter;
                sim.advance_space(
                    e,
                    time,
                    gates(camera.height, &assets.graphics).2,
                    Some(motion::Space::new(
                        e.local_space,
                        e.system_local_space,
                        e.scale,
                        Transform {
                            translation: sim.anchor,
                            ..Transform::IDENTITY
                        },
                    )),
                );
                let mut packet = DrawPacket::weather(e, sim, v.anchor, v.alpha);
                packet.actor = [0, v.key.0, usize::from(v.key.1), e.actor];
                packet.views = v.views;
                expected.push(packet);
            }
            for (key, sim) in &mut serial {
                if !visible.contains(key) {
                    sim.time = time.max(0.);
                }
            }
            let mut actual = Vec::new();
            let mut drawn = super::super::Drawn::default();
            let mut fades = super::super::fade::Fades::default();
            fades.produce([7], camera.height, &assets.graphics, |_| true);
            weather_packets(
                &assets,
                drawn.for_field(&assets.anchors, &assets.systems, &field, &presentation, 1),
                (camera, &views, time),
                &fades,
                &mut made,
                &mut actual,
            );
            assert_eq!(
                serial.keys().collect::<Vec<_>>(),
                made.keys().collect::<Vec<_>>()
            );
            for (key, sim) in &serial {
                super::super::tests::assert_same_state(sim, &made[key]);
            }
            assert_packets_equal(&expected, &actual);
            order(&mut expected);
            order(&mut actual);
            assert_packets_equal(&expected, &actual);
        }
    }

    #[test]
    fn settled_hulls_keep_one_simulation_per_persistent_event() {
        use viewer_models::{self as models, entity_animation};
        use viewer_unit_history::unit_history;
        let mut rig = entity_animation::stage_rig(false);
        rig.nodes[0].clips[0].animation = None;
        rig.nodes[0].clips[0]
            .state
            .events
            .push(content_reader_mesh::mesh::EntityEvent {
                particle: Some("wake".into()),
                keep_particle: true,
                ..Default::default()
            });
        let rigs = [Arc::new(rig)];
        let mut emitter = super::super::test_support::activation_test_emitter();
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
                assert_eq!(simulations.0["wake"].len(), 2, "one runtime per hull event");
                if first.is_empty() {
                    let mut duplicate = events.clone();
                    duplicate.push(events[0].clone());
                    let refusal = simulations.prepare(&entities, &duplicate).unwrap_err();
                    assert!(refusal.to_string().starts_with(
                        "Map shader particles: 1 refused actor wake: duplicate presentation identity"
                    ));
                    first = events
                        .iter()
                        .map(|event| (event.key.clone(), event.age))
                        .collect();
                }
                let mut current = Vec::new();
                for (event, (key, age)) in events.iter().zip(&first) {
                    assert_eq!(&event.key, key, "reapplying idle keeps the live event");
                    assert!((event.age - age - (time - 1.)).abs() < 1e-5);
                    let sim = &simulations.0["wake"][&(key.clone(), 0)].simulation;
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
        let mut emitter = super::super::test_support::activation_test_emitter();
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
        let constants = DrawConstants::new(&packet.constants);
        let object = &constants.object;
        assert_eq!(&object[72..80], bytemuck::bytes_of(&[2_f32, 2.]));
    }

    #[test]
    fn canonical_packets_use_seam_carriers_without_advancing_births() {
        use viewer_gpu::camera::{Camera, MapDefines, MapSize};
        let size = [1200, 800];
        let camera = Camera {
            center: [5., 400.],
            height: 600.,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: viewer_gpu::camera::Pose::Map,
        };
        let anchor = [camera.map.x() - 5., 10., 400.];
        for local in [false, true] {
            let mut emitter = super::super::test_support::activation_test_emitter();
            emitter.local_space = local;
            let mut simulation = Simulation::cold(&emitter, 7, anchor, 0.);
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
                let _ = DrawConstants::new(&packet.constants);
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
    fn sorting_once_then_filtering_matches_each_views_native_stable_order() {
        let mut emitter = super::super::test_support::activation_test_emitter();
        let mut packets = Vec::new();
        for (index, (registration, actor, trail)) in [
            (2, 1, true),
            (0, 2, false),
            (2, 1, false),
            (0, 2, false),
            (1, 0, true),
        ]
        .into_iter()
        .enumerate()
        {
            emitter.registration = registration;
            emitter.render.trail = trail;
            let mut packet = DrawPacket::new(&emitter, 1., Transform::IDENTITY, index as f32);
            packet.actor = [0, actor, 0, 0];
            packet.views = ViewMask::Some((0..12).map(|view| (index + view) % 3 != 0).collect());
            packets.push(packet);
        }
        let before: Vec<Vec<_>> = (0..13)
            .map(|view| {
                submission_order(&packets, view)
                    .iter()
                    .map(|packet| packet.constants.time.to_bits())
                    .collect()
            })
            .collect();
        order(&mut packets);
        for (view, expected) in before.iter().enumerate() {
            let actual: Vec<_> = packets
                .iter()
                .filter(|packet| packet.views.contains(view))
                .map(|packet| packet.constants.time.to_bits())
                .collect();
            assert_eq!(&actual, expected);
        }
    }

    #[test]
    fn type_grouping_preserves_actor_and_subsystem_order() {
        let mut emitter = super::super::test_support::activation_test_emitter();
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
        let mut emitter = super::super::test_support::activation_test_emitter();
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
        let mut emitter = super::super::test_support::activation_test_emitter();
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
        assert_eq!(
            packets.iter().map(|p| p.views.clone()).collect::<Vec<_>>(),
            [second.clone(), second, third.clone(), third]
        );
    }

    #[test]
    fn view_masks_hold_any_number_of_views() {
        let mut views = vec![false; 12];
        views[9..].fill(true);
        let mask = ViewMask::Some(views);
        assert!(!mask.contains(8) && mask.contains(9) && mask.contains(11));
        assert!(!mask.contains(12));
        assert!(!ViewMask::Some(vec![false; 3]).contains(0) && !ViewMask::Some(Vec::new()).contains(0));
        assert!(ViewMask::All.contains(1_000));
        let emitter = super::super::test_support::activation_test_emitter();
        let mut packet = DrawPacket::new(&emitter, 1., Transform::IDENTITY, 1.);
        packet.views = mask;
        let packets = [packet];
        assert!(submission_order(&packets, 8).is_empty());
        assert_eq!(submission_order(&packets, 10).len(), 1);
    }
}

#[cfg(test)]
#[path = "native/draw_tests.rs"]
mod draw_tests;

fn particle_depth() -> content_reader_gui::pdxshader::DepthStencilState {
    content_reader_gui::pdxshader::DepthStencilState {
        depth_enable: true,
        depth_write_mask: 0,
        depth_function: 3,
        stencil_enable: false,
        ..Default::default()
    }
}

fn particle_pass(samples: u32, topology: wgpu::PrimitiveTopology) -> gpu::Pass {
    let depth = particle_depth();
    gpu::Pass {
        vertices: Vec::new(),
        primitive: wgpu::PrimitiveState {
            topology,
            ..Default::default()
        },
        depth_stencil: Some(
            gpu::depth_stencil(&depth, &Default::default(), gpu_contract::DEPTH_FORMAT).0,
        ),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        targets: vec![Some(wgpu::ColorTargetState {
            format: gpu_contract::SCENE_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })],
        instanced: 0,
        stream: None,
        vertex_color_tint: None,
    }
}
