use crate::mod_resolver::ContentAdmissionLease;
use crate::particle_authored::{
    AdmittedParticleDefinition, AuthoredSubsystem, ParticleEmitterShape, ParticleForceKind,
    ParticleInputRefusal, ParticleRefusalReason, ParticleResult, ParticleSort, ParticleTexture,
    VariedScalar,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

static NEXT_RUNTIME: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerParticleSeed {
    pub counter: u32,
    pub seed: u32,
}

impl ViewerParticleSeed {
    fn draw(&mut self) -> u32 {
        let mut value = self
            .seed
            .wrapping_add(self.counter.wrapping_mul(0xb5297a4d));
        value = ((value >> 8) ^ value).wrapping_add(0x68e31da4);
        value = (value.wrapping_mul(0x100) ^ value).wrapping_mul(0x1b56c4e9);
        value = ((value >> 8) ^ value).wrapping_mul(0x92d68ca2);
        self.counter = self.counter.wrapping_add(1);
        ((value >> 8) ^ value) & 0x7fff_ffff
    }

    fn initialize_turbulence(&mut self) {
        // Noise vectors use rejection sampling; skipping a nominal number of draws changes life.
        for _ in 0..256 {
            for _ in 0..2 {
                loop {
                    let mut vector = [0.0_f32; 3];
                    for coordinate in &mut vector {
                        let half = self.draw() as f32 * f32::from_bits(0x3000_0000) - 0.5;
                        *coordinate = half + half;
                    }
                    let square =
                        vector[1] * vector[1] + vector[0] * vector[0] + vector[2] * vector[2];
                    if square <= 1.0 && square >= f32::from_bits(0x3a83_126f) {
                        break;
                    }
                }
            }
            self.draw();
        }
        for _ in 0..255 {
            self.draw();
        }
    }

    fn sample(&mut self, value: &VariedScalar, time: f32) -> f32 {
        let (base, range, random) = value.initial(time);
        if !random {
            return base;
        }
        let unit = self.draw() as f32 * f32::from_bits(0x3000_0000);
        unit * (range + range) + (base - range)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleHandle {
    runtime: u64,
    generation: u64,
    slot: usize,
    event_instance_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ParticleLightBinding {
    pub event_instance_id: u64,
    pub presentation_id: Option<u64>,
    pub world_matrix_f32_bits: [u32; 16],
    pub finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleSortCamera {
    pub position_f32_bits: [u32; 3],
    pub forward_f32_bits: [u32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleGpuRecord(pub [u8; 48]);

#[derive(Clone, Copy, Debug)]
pub struct ParticleGpuView<'a> {
    instance: &'a Instance,
    pub presentation_id: Option<u64>,
}

impl<'a> ParticleGpuView<'a> {
    pub fn handle(self) -> ParticleHandle {
        self.instance.handle
    }
    pub fn definition(self) -> &'a AdmittedParticleDefinition {
        &self.instance.definition
    }
    pub fn content_lease(self) -> &'a ContentAdmissionLease {
        &self.instance.definition.lease
    }
    pub fn world_matrix_f32_bits(self) -> [u32; 16] {
        self.instance.world
    }
    pub fn sorted_presentation_id(self) -> Option<u64> {
        self.instance.last_sort.map(|(id, _)| id)
    }
    pub fn draw_scale_f32_bits(self) -> ParticleResult<[u32; 2]> {
        let scale = self.instance.effective_scale()?;
        Ok([
            scale.to_bits(),
            if self
                .instance
                .definition
                .subsystems
                .iter()
                .any(|system| system.local_space)
            {
                scale.to_bits()
            } else {
                1.0_f32.to_bits()
            },
        ])
    }

    pub fn root_subsystems(self) -> impl ExactSizeIterator<Item = ParticleGpuSubsystem<'a>> {
        self.instance
            .subsystems
            .iter()
            .zip(self.instance.definition.subsystems.iter())
            .map(move |(state, authored)| ParticleGpuSubsystem {
                instance: self.instance,
                state,
                authored,
                local_frame: ParticleLocalFrame::Root,
                presentation_id: self.presentation_id,
            })
    }

    pub fn visit_draw_subsystems(
        self,
        packet_capacity: usize,
        trail: bool,
        mut visit: impl FnMut(usize, ParticleGpuSubsystem<'a>) -> crate::Result<()>,
    ) -> crate::Result<usize> {
        if packet_capacity > i32::MAX as usize {
            return Err(runtime_refusal(
                "particle_draw_packet_capacity",
                ParticleRefusalReason::InvalidNumber,
            )
            .event(self.handle().event_instance_id)
            .into());
        }
        let mut ordinal = 0;
        for root in self.root_subsystems() {
            root.visit_draw_subsystems(packet_capacity, &mut ordinal, trail, &mut visit)?;
        }
        Ok(ordinal)
    }
}

#[derive(Clone, Copy, Debug)]
enum ParticleLocalFrame {
    Root,
    Child([f32; 3]),
}

#[derive(Clone, Copy, Debug)]
pub struct ParticleGpuSubsystem<'a> {
    instance: &'a Instance,
    state: &'a Subsystem,
    authored: &'a AuthoredSubsystem,
    local_frame: ParticleLocalFrame,
    presentation_id: Option<u64>,
}

impl<'a> ParticleGpuSubsystem<'a> {
    pub(crate) fn content_lease(self) -> &'a ContentAdmissionLease {
        &self.instance.definition.lease
    }

    pub(crate) fn validate_draw_presentation(
        self,
        presentation_id: u64,
        camera: ParticleSortCamera,
    ) -> ParticleResult<()> {
        if self.presentation_id != Some(presentation_id)
            || self.instance.last_sort != Some((presentation_id, camera))
        {
            return Err(ParticleInputRefusal::new(
                self.instance.definition.name(),
                Some(self.name()),
                "particle_draw_presentation",
                ParticleRefusalReason::PresentationOutOfOrder,
            )
            .event(self.instance.handle.event_instance_id));
        }
        Ok(())
    }

    pub fn draw_world_matrix_f32_bits(self) -> ParticleResult<[u32; 16]> {
        let mut matrix = [
            1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        if self.authored.local_space {
            match self.local_frame {
                ParticleLocalFrame::Root => {
                    matrix = self.instance.world.map(f32::from_bits);
                    for base in [0, 4, 8] {
                        let length = (matrix[base] * matrix[base]
                            + matrix[base + 1] * matrix[base + 1]
                            + matrix[base + 2] * matrix[base + 2])
                            .sqrt();
                        if length != 0.0 {
                            let inverse = 1.0 / length;
                            for value in &mut matrix[base..base + 4] {
                                *value *= inverse;
                            }
                        }
                    }
                }
                ParticleLocalFrame::Child(position) => matrix[12..15].copy_from_slice(&position),
            }
        }
        if matrix.iter().any(|value| !value.is_finite()) {
            return Err(ParticleInputRefusal::new(
                self.instance.definition.name(),
                Some(self.name()),
                "particle_draw_matrix",
                ParticleRefusalReason::ArithmeticOverflow,
            )
            .event(self.instance.handle.event_instance_id));
        }
        Ok(matrix.map(f32::to_bits))
    }

    pub fn visit_trail_vertices(
        self,
        camera_position_f32_bits: [u32; 3],
        scale_f32_bits: u32,
        mut visit: impl FnMut([u8; 28]) -> ParticleResult<()>,
    ) -> ParticleResult<()> {
        let fail = |input, reason| {
            ParticleInputRefusal::new(
                self.instance.definition.name(),
                Some(self.name()),
                input,
                reason,
            )
            .event(self.instance.handle.event_instance_id)
        };
        if !self.trail() {
            return Err(fail(
                "trail_vertices",
                ParticleRefusalReason::UnsupportedAuthoredInput,
            ));
        }
        let camera = camera_position_f32_bits.map(f32::from_bits);
        let scale = f32::from_bits(scale_f32_bits);
        if !scale.is_finite() || (self.billboard() && camera.iter().any(|v| !v.is_finite())) {
            return Err(fail(
                "trail_camera_scale",
                ParticleRefusalReason::InvalidNumber,
            ));
        }
        if self.live_count() < 2 {
            return Ok(());
        }
        let normalize = |mut vector: [f32; 3]| {
            let length =
                (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
            if length != 0.0 {
                for value in &mut vector {
                    *value /= length;
                }
            }
            vector
        };
        let mut records = self.records();
        let mut previous = records.next().unwrap()?;
        let position = |record: &ParticleGpuRecord| {
            std::array::from_fn::<_, 3, _>(|axis| {
                f32::from_le_bytes(record.0[axis * 4..axis * 4 + 4].try_into().unwrap())
            })
        };
        let mut previous_direction = [0.0; 3];
        for (index, next) in records.enumerate() {
            let next = next?;
            let start = position(&previous);
            let end = position(&next);
            let delta: [f32; 3] = std::array::from_fn(|axis| end[axis] - start[axis]);
            let mut direction = if self.billboard() {
                let eye: [f32; 3] = std::array::from_fn(|axis| camera[axis] - start[axis]);
                normalize([
                    eye[1] * delta[2] - eye[2] * delta[1],
                    eye[2] * delta[0] - eye[0] * delta[2],
                    eye[0] * delta[1] - eye[1] * delta[0],
                ])
            } else {
                normalize([
                    delta[1] * 0.0 - delta[2],
                    delta[2] * 0.0 - delta[0] * 0.0,
                    delta[0] - delta[1] * 0.0,
                ])
            };
            if !self.billboard() {
                let current = direction;
                if index != 0 {
                    direction = normalize(std::array::from_fn(|axis| {
                        (previous_direction[axis] + current[axis]) * 0.5
                    }));
                }
                previous_direction = current;
            }
            let mut pair = |record: &ParticleGpuRecord, ordinal: usize| -> ParticleResult<()> {
                let center = position(record);
                let size = f32::from_le_bytes(record.0[12..16].try_into().unwrap());
                for positive in [true, false] {
                    let vertex: [f32; 3] = std::array::from_fn(|axis| {
                        let offset = size * direction[axis];
                        (if positive {
                            offset + center[axis]
                        } else {
                            center[axis] - offset
                        }) * scale
                    });
                    if vertex.iter().any(|value| !value.is_finite()) {
                        return Err(fail(
                            "trail_vertex_position",
                            ParticleRefusalReason::ArithmeticOverflow,
                        ));
                    }
                    let mut bytes = [0_u8; 28];
                    for (axis, value) in vertex.iter().enumerate() {
                        bytes[axis * 4..axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
                    }
                    bytes[12..16].copy_from_slice(&(ordinal as f32).to_le_bytes());
                    bytes[16..20]
                        .copy_from_slice(&(if positive { 1.0_f32 } else { 0.0 }).to_le_bytes());
                    bytes[20..24].copy_from_slice(&record.0[28..32]);
                    bytes[24..28].copy_from_slice(&record.0[32..36]);
                    visit(bytes)?;
                }
                Ok(())
            };
            pair(&previous, index)?;
            if index + 2 == self.live_count() {
                pair(&next, index + 1)?;
            }
            previous = next;
        }
        Ok(())
    }

    fn visit_draw_subsystems(
        self,
        packet_capacity: usize,
        ordinal: &mut usize,
        trail: bool,
        visit: &mut impl FnMut(usize, ParticleGpuSubsystem<'a>) -> crate::Result<()>,
    ) -> crate::Result<()> {
        if *ordinal == packet_capacity {
            return Ok(());
        }
        // Both partitions and zero-live entries consume the native shared packet bound.
        let current = *ordinal;
        *ordinal += 1;
        if self.trail() == trail {
            visit(current, self)?;
        }
        for (_, _, child) in self.children() {
            child.visit_draw_subsystems(packet_capacity, ordinal, trail, visit)?;
        }
        Ok(())
    }

    pub fn name(self) -> &'a str {
        &self.authored.name
    }
    pub fn texture(self) -> Option<&'a ParticleTexture> {
        self.authored.texture.as_ref()
    }
    pub fn sort(self) -> ParticleSort {
        self.authored.sort
    }
    pub fn invert(self) -> bool {
        self.authored.invert
    }
    pub fn local_space(self) -> bool {
        self.authored.local_space
    }
    pub fn billboard(self) -> bool {
        self.authored.billboard
    }
    pub fn trail(self) -> bool {
        self.authored.trail
    }
    pub fn local_time_bits(self) -> u32 {
        self.state.local_time.to_bits()
    }
    pub fn live_count(self) -> usize {
        self.state.live
    }
    pub fn authored_capacity(self) -> usize {
        self.authored.capacity
    }

    // These are runtime storage indices; the render sort determines submission order.
    pub fn children(self) -> impl Iterator<Item = (usize, usize, ParticleGpuSubsystem<'a>)> {
        self.state.particles[..self.state.retained]
            .iter()
            .enumerate()
            .flat_map(move |(parent_index, particle)| {
                let count = self.authored.children.len();
                let first = particle.child_slot * count;
                self.state.children[first..first + count]
                    .iter()
                    .zip(self.authored.children.iter())
                    .enumerate()
                    .map(move |(child_index, (state, authored))| {
                        (
                            parent_index,
                            child_index,
                            ParticleGpuSubsystem {
                                instance: self.instance,
                                state,
                                authored,
                                local_frame: ParticleLocalFrame::Child(particle.visual.position),
                                presentation_id: self.presentation_id,
                            },
                        )
                    })
            })
    }

    pub fn records(self) -> impl ExactSizeIterator<Item = ParticleResult<ParticleGpuRecord>> + 'a {
        self.state.particles[..self.state.live]
            .iter()
            .map(move |particle| {
                let fail = |input| {
                    ParticleInputRefusal::new(
                        self.instance.definition.name(),
                        Some(self.name()),
                        input,
                        ParticleRefusalReason::ArithmeticOverflow,
                    )
                    .event(self.instance.handle.event_instance_id)
                };
                let visual = &particle.visual;
                let mut bytes = [0; 48];
                for (index, value) in visual
                    .position
                    .iter()
                    .chain(visual.columns[..4].iter())
                    .enumerate()
                {
                    if !value.is_finite() {
                        return Err(fail("gpu_position_size_rotation"));
                    }
                    bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_bits().to_le_bytes());
                }
                if visual.born {
                    let dimensions = self
                        .texture()
                        .map_or([1, 1], |texture| [texture.columns, texture.rows]);
                    for axis in 0..2 {
                        if dimensions[axis] == 0 {
                            return Err(fail("texture_atlas_dimension"));
                        }
                        bytes[0x1c + axis] =
                            ((visual.atlas_draws[axis] as i32) % dimensions[axis]) as u8;
                    }
                }
                bytes[0x20..0x24].copy_from_slice(&visual.gpu_color);
                Ok(ParticleGpuRecord(bytes))
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleAction {
    Continue,
    StopEmission,
    StopUnboundedEmission,
    StopAndDetach,
    Detach,
    Destroy,
}

#[derive(Clone, Copy, Debug)]
pub struct ParticleInstanceUpdate {
    pub handle: ParticleHandle,
    pub parent_world_matrix_f32_bits: Option<[u32; 16]>,
    pub inherited_speed: Option<f32>,
    pub action: ParticleAction,
}

fn native_angles(
    authored: &AuthoredSubsystem,
    input: &str,
    radians: f32,
) -> ParticleResult<(f32, f32)> {
    use portable_render_stream::exact_math::{native_cos, native_sin, viewer_uses_fma};
    let fma = viewer_uses_fma();
    native_sin(radians, fma)
        .and_then(|sin| native_cos(radians, fma).map(|cos| (sin, cos)))
        .map_err(|_| {
            ParticleInputRefusal::new(
                "",
                Some(&authored.name),
                input,
                ParticleRefusalReason::ArithmeticOverflow,
            )
        })
}

#[derive(Clone, Copy, Debug, Default)]
struct ParticleVisualState {
    birth_columns: [f32; 12],
    columns: [f32; 12],
    atlas_draws: [u32; 2],
    gpu_color: [u8; 4],
    born: bool,
    emitter_angles: [f32; 2],
    velocity_angles_and_speed: [f32; 3],
    shape_coordinates: [f32; 3],
    position: [f32; 3],
    velocity: [f32; 3],
}

impl ParticleVisualState {
    fn initialize(&mut self) {
        // The birth writer clamps both the CPU color samples and uploaded colors.
        for color in &mut self.birth_columns[8..] {
            *color = if 0.0 <= *color { color.min(255.0) } else { 0.0 };
        }
        self.columns = self.birth_columns;
        self.gpu_color = std::array::from_fn(|channel| self.columns[8 + channel] as u8);
        self.born = true;
    }

    fn copy_gpu_from(&mut self, source: Self) {
        self.position = source.position;
        self.columns[..4].copy_from_slice(&source.columns[..4]);
        self.columns[8..].copy_from_slice(&source.columns[8..]);
        self.atlas_draws = source.atlas_draws;
        self.gpu_color = source.gpu_color;
        self.born = source.born;
    }

    fn initialize_motion(
        &mut self,
        authored: &AuthoredSubsystem,
        world: &[f32; 16],
        previous_translation: [f32; 3],
        origin: [f32; 3],
        index: usize,
        amount: usize,
        scale: f32,
    ) -> ParticleResult<()> {
        let negative_radians = f32::from_bits(0xbc8e_fa35);
        let yaw = ((self.emitter_angles[0] + f32::from_bits(0x42b4_0000))
            * f32::from_bits(0xc049_0fdb))
            / f32::from_bits(0x4334_0000);
        let pitch = self.emitter_angles[1] * negative_radians;
        let (sy, cy) = native_angles(authored, "emitter_yaw", yaw)?;
        let (sp, cp) = native_angles(authored, "emitter_pitch", pitch)?;
        let first = std::array::from_fn::<_, 3, _>(|axis| {
            world[4 + axis] * -sy + (cp * cy) * world[axis] + world[8 + axis] * (sp * cy)
        });
        let second = std::array::from_fn::<_, 3, _>(|axis| {
            world[4 + axis] * cy + (cp * sy) * world[axis] + world[8 + axis] * (sp * sy)
        });
        let third = std::array::from_fn::<_, 3, _>(|axis| {
            -sp * world[axis] + world[4 + axis] * 0.0 + world[8 + axis] * cp
        });
        let velocity_pitch = self.velocity_angles_and_speed[0] * negative_radians;
        let velocity_yaw = self.velocity_angles_and_speed[1] * negative_radians;
        let (sin_pitch, cos_pitch) = native_angles(authored, "velocity_pitch", velocity_pitch)?;
        let (sin_yaw, cos_yaw) = native_angles(authored, "velocity_yaw", velocity_yaw)?;
        let x = cos_yaw * cos_pitch;
        let y = -sin_pitch;
        let z = sin_yaw * cos_pitch;
        let speed = self.velocity_angles_and_speed[2] * scale;
        self.velocity = std::array::from_fn(|axis| {
            (y * second[axis] + x * first[axis] + z * third[axis]) * speed
        });
        let shape = match &authored.shape {
            ParticleEmitterShape::Point => [0.0; 3],
            ParticleEmitterShape::Box(_) => self.shape_coordinates,
            ParticleEmitterShape::Sphere(_) => {
                let [radius, yaw, pitch] = self.shape_coordinates;
                let pitch = pitch * negative_radians;
                let yaw = yaw * negative_radians;
                let (sin_pitch, cos_pitch) = native_angles(authored, "sphere_pitch", pitch)?;
                let (sin_yaw, cos_yaw) = native_angles(authored, "sphere_yaw", yaw)?;
                [
                    cos_yaw * cos_pitch * radius,
                    -sin_pitch * radius,
                    sin_yaw * cos_pitch * radius,
                ]
            }
        };
        let shape = if matches!(&authored.shape, ParticleEmitterShape::Point) {
            shape
        } else {
            std::array::from_fn(|axis| {
                shape[1] * second[axis] + shape[0] * first[axis] + shape[2] * third[axis]
            })
        };
        let fraction = index as f32 / amount as f32;
        self.position = std::array::from_fn(|axis| {
            (world[12 + axis] - previous_translation[axis]) * fraction
                + previous_translation[axis]
                + origin[axis]
                + shape[axis]
        });
        Ok(())
    }

    fn advance(
        &mut self,
        authored: &AuthoredSubsystem,
        time: f32,
        age: f32,
        life: f32,
        delta: f32,
        opacity: f32,
    ) {
        for (index, property) in authored.spawn_draw_columns.iter().enumerate() {
            self.columns[index] = property.update_particle(
                self.birth_columns[index],
                self.columns[index],
                time,
                age,
                life,
            );
        }
        for axis in 0..3 {
            self.columns[1 + axis] = delta * self.columns[4 + axis] + self.columns[1 + axis];
        }
        for channel in 0..4 {
            if authored.spawn_draw_columns[8 + channel].has_particle_update() {
                self.gpu_color[channel] = self.columns[8 + channel] as u8;
            }
        }
        if opacity < 1.0 {
            let alpha = self.gpu_color[3] as f32 * opacity;
            let rounded = if 0.0 <= alpha {
                alpha + 0.5
            } else {
                alpha - 0.5
            };
            self.gpu_color[3] = rounded as u8;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LiveParticle {
    age: f32,
    life: f32,
    child_slot: usize,
    visual: ParticleVisualState,
}

#[derive(Debug)]
struct Subsystem {
    origin: [f32; 3],
    previous_translation: [f32; 3],
    local_time: f32,
    end: f32,
    fractional_emission: f32,
    pulse_duration: f32,
    pulse_silence: f32,
    pulse_time: f32,
    pulse_on: bool,
    particles: Vec<LiveParticle>,
    live: usize,
    retained: usize,
    children: Vec<Subsystem>,
    force_scratch: Vec<[f32; 3]>,
    sort_phase: usize,
}

impl Subsystem {
    fn new(authored: &AuthoredSubsystem, random: &mut ViewerParticleSeed) -> ParticleResult<Self> {
        let mut particles = Vec::new();
        particles
            .try_reserve_exact(authored.capacity)
            .map_err(|_| {
                ParticleInputRefusal::new(
                    "",
                    Some(&authored.name),
                    "max_amount",
                    ParticleRefusalReason::CapacityExceeded,
                )
            })?;
        let child_count = authored
            .capacity
            .checked_mul(authored.children.len())
            .ok_or_else(|| {
                ParticleInputRefusal::new(
                    "",
                    Some(&authored.name),
                    "childsystem",
                    ParticleRefusalReason::CapacityExceeded,
                )
            })?;
        let mut children = Vec::new();
        children.try_reserve_exact(child_count).map_err(|_| {
            ParticleInputRefusal::new(
                "",
                Some(&authored.name),
                "childsystem",
                ParticleRefusalReason::CapacityExceeded,
            )
        })?;
        for slot in 0..authored.capacity {
            particles.push(LiveParticle {
                age: 0.0,
                life: 0.0,
                child_slot: slot,
                visual: ParticleVisualState::default(),
            });
            for child in &authored.children {
                children.push(Self::new(child, random)?);
            }
        }
        let origin = std::array::from_fn(|axis| random.sample(&authored.origin[axis], 0.0));
        // 1423dcc80 evaluates end before delay and negates delay by flipping its sign bit.
        let end = random.sample(&authored.duration, 0.0);
        let local_time =
            f32::from_bits(random.sample(&authored.delay, 0.0).to_bits() ^ 0x8000_0000);
        let pulse_duration = random.sample(&authored.pulse_duration, 0.0);
        let pulse_silence = random.sample(&authored.pulse_silence, 0.0);
        if [local_time, end, pulse_duration, pulse_silence]
            .iter()
            .any(|v| !v.is_finite())
        {
            return Err(ParticleInputRefusal::new(
                "",
                Some(&authored.name),
                "initial_values",
                ParticleRefusalReason::ArithmeticOverflow,
            ));
        }
        let mut force_scratch = Vec::new();
        if !authored.forces.is_empty() {
            force_scratch
                .try_reserve_exact(authored.capacity.max(1))
                .map_err(|_| {
                    ParticleInputRefusal::new(
                        "",
                        Some(&authored.name),
                        "force",
                        ParticleRefusalReason::CapacityExceeded,
                    )
                })?;
            force_scratch.resize(authored.capacity.max(1), [0.0; 3]);
        }
        Ok(Self {
            origin,
            previous_translation: origin,
            local_time,
            end,
            fractional_emission: 0.0,
            pulse_duration,
            pulse_silence,
            pulse_time: 0.0,
            pulse_on: true,
            particles,
            live: 0,
            retained: 0,
            children,
            force_scratch,
            sort_phase: 1,
        })
    }

    fn allocated_capacity(&self) -> usize {
        self.particles.capacity()
            + self
                .children
                .iter()
                .map(Self::allocated_capacity)
                .sum::<usize>()
    }

    fn finished(&self) -> bool {
        !(self.end < 0.0 || self.local_time <= self.end || self.retained != 0)
    }

    fn sort_for_draw(
        &mut self,
        authored: &AuthoredSubsystem,
        camera: ParticleSortCamera,
    ) -> ParticleResult<()> {
        if self.live != 0 && authored.sort != ParticleSort::None {
            let gap = [1, 4, 1, 16, 1, 64][self.sort_phase];
            self.sort_phase = (self.sort_phase + 1) % 6;
            let key = |particle: &LiveParticle| -> ParticleResult<f32> {
                let relative: [f32; 3] = std::array::from_fn(|axis| {
                    particle.visual.position[axis] - f32::from_bits(camera.position_f32_bits[axis])
                });
                let value = match authored.sort {
                    ParticleSort::None => unreachable!(),
                    ParticleSort::Age => particle.age,
                    ParticleSort::Depth => {
                        relative[0] * f32::from_bits(camera.forward_f32_bits[0])
                            + relative[1] * f32::from_bits(camera.forward_f32_bits[1])
                            + relative[2] * f32::from_bits(camera.forward_f32_bits[2])
                    }
                    ParticleSort::Distance => {
                        relative[1] * relative[1]
                            + relative[0] * relative[0]
                            + relative[2] * relative[2]
                    }
                };
                if !value.is_finite() {
                    return Err(ParticleInputRefusal::new(
                        "",
                        Some(&authored.name),
                        "particle_sort_key",
                        ParticleRefusalReason::ArithmeticOverflow,
                    ));
                }
                Ok(value)
            };
            // 1423dfd30 performs two gap passes with a strict end bound, not a complete sort.
            for first_end in [gap * 2, gap * 3] {
                for end in (first_end..self.live).step_by(gap * 2) {
                    for left in end - gap * 2..end - gap {
                        let right = left + gap;
                        let a = key(&self.particles[left])?;
                        let b = key(&self.particles[right])?;
                        if if authored.invert { b < a } else { a < b } {
                            self.particles.swap(left, right);
                        }
                    }
                }
            }
        }
        let count = authored.children.len();
        for parent in &self.particles[..self.retained] {
            let first = parent.child_slot * count;
            for (state, definition) in self.children[first..first + count]
                .iter_mut()
                .zip(&authored.children)
            {
                state.sort_for_draw(definition, camera)?;
            }
        }
        Ok(())
    }

    fn swap_cpu_copy_gpu(&mut self, source: usize, destination: usize) {
        // The native CPU record swaps; the GPU source record stays in place.
        let gpu_source = self.particles[source].visual;
        self.particles.swap(source, destination);
        self.particles[source].visual.copy_gpu_from(gpu_source);
    }

    fn advance_motion(
        &mut self,
        authored: &AuthoredSubsystem,
        delta: f32,
        scale: f32,
        world: &[f32; 16],
        random: &mut ViewerParticleSeed,
    ) -> ParticleResult<()> {
        let scaled_delta = scale * delta;
        if authored.forces.is_empty() {
            for particle in &mut self.particles[..self.live] {
                for axis in 0..3 {
                    particle.visual.position[axis] = particle.visual.velocity[axis] * scaled_delta
                        + particle.visual.position[axis];
                }
            }
            return self.validate_motion(authored);
        }
        let sample_count = if authored.force_division > 0 {
            self.live / (authored.force_division as usize + 1) + 1
        } else {
            self.live
        };
        let forces = &mut self.force_scratch[..sample_count];
        forces.fill([0.0; 3]);
        for force in &authored.forces {
            let amount = random.sample(&force.amount, self.local_time);
            if amount == 0.0 {
                continue;
            }
            match force.kind {
                ParticleForceKind::Planar => {
                    for accumulated in forces.iter_mut() {
                        for axis in 0..3 {
                            accumulated[axis] = force.direction[axis] * amount + accumulated[axis];
                        }
                    }
                }
                ParticleForceKind::Point | ParticleForceKind::Vortex => {
                    let rotation = if force.kind == ParticleForceKind::Vortex && force.yaw != 0.0 {
                        Some(native_angles(
                            authored,
                            "force_yaw",
                            (force.yaw * f32::from_bits(0x4049_0fdb)) / f32::from_bits(0x4334_0000),
                        )?)
                    } else {
                        None
                    };
                    let origin = std::array::from_fn::<_, 3, _>(|axis| {
                        force.position[axis] * scale
                            + if force.local_force {
                                world[12 + axis]
                            } else {
                                0.0
                            }
                    });
                    let strength = amount * scale;
                    for (accumulated, particle) in forces.iter_mut().zip(&self.particles) {
                        let mut radial = std::array::from_fn::<_, 3, _>(|axis| {
                            particle.visual.position[axis] - origin[axis]
                        });
                        if force.kind == ParticleForceKind::Vortex {
                            let projection = force.direction[0] * radial[0]
                                + force.direction[1] * radial[1]
                                + force.direction[2] * radial[2];
                            for axis in 0..3 {
                                radial[axis] -= force.direction[axis] * projection;
                            }
                        }
                        let length =
                            (radial[0] * radial[0] + radial[1] * radial[1] + radial[2] * radial[2])
                                .sqrt();
                        if f32::from_bits(0x3c23_d70a) < length {
                            let factor = strength / length;
                            if let Some((sin, cos)) = rotation {
                                let [x, y, z] = radial;
                                let [dx, dy, dz] = force.direction;
                                accumulated[0] =
                                    ((dz * y - dy * z) * sin + x * cos) * factor + accumulated[0];
                                accumulated[1] =
                                    ((dx * z - dz * x) * sin + cos * y) * factor + accumulated[1];
                                accumulated[2] =
                                    ((dy * x - dx * y) * sin + cos * z) * factor + accumulated[2];
                            } else {
                                for axis in 0..3 {
                                    accumulated[axis] = factor * radial[axis] + accumulated[axis];
                                }
                            }
                        }
                    }
                }
                ParticleForceKind::Friction => {
                    for (accumulated, particle) in forces.iter_mut().zip(&self.particles) {
                        for axis in 0..3 {
                            accumulated[axis] -= particle.visual.velocity[axis] * amount;
                        }
                    }
                }
                _ => {
                    return Err(ParticleInputRefusal::new(
                        "",
                        Some(&authored.name),
                        &format!("force:{}:{:?}", force.name, force.kind),
                        ParticleRefusalReason::UnsupportedAuthoredInput,
                    ))
                }
            }
        }
        for (index, particle) in self.particles[..self.live].iter_mut().enumerate() {
            let acceleration_delta = delta / particle.visual.columns[7];
            for axis in 0..3 {
                particle.visual.velocity[axis] = forces[index % sample_count][axis]
                    * acceleration_delta
                    + particle.visual.velocity[axis];
                particle.visual.position[axis] =
                    particle.visual.velocity[axis] * scaled_delta + particle.visual.position[axis];
            }
        }
        self.validate_motion(authored)
    }

    fn validate_motion(&self, authored: &AuthoredSubsystem) -> ParticleResult<()> {
        if self.particles[..self.live].iter().any(|particle| {
            particle
                .visual
                .position
                .iter()
                .chain(&particle.visual.velocity)
                .any(|v| !v.is_finite())
        }) {
            return Err(ParticleInputRefusal::new(
                "",
                Some(&authored.name),
                "particle_motion",
                ParticleRefusalReason::ArithmeticOverflow,
            ));
        }
        Ok(())
    }

    fn advance(
        &mut self,
        authored: &AuthoredSubsystem,
        delta: f32,
        random: &mut ViewerParticleSeed,
        parent_alive: bool,
        parent_world: [f32; 16],
        instance_scale: f32,
        opacity: f32,
    ) -> ParticleResult<()> {
        let world = if authored.local_space {
            [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ]
        } else {
            parent_world
        };
        let translation = [world[12], world[13], world[14]];
        let scale = if authored.local_space {
            1.0
        } else {
            instance_scale
        };
        if self.live == 0 {
            self.previous_translation = translation;
        }
        let fail = || {
            ParticleInputRefusal::new(
                "",
                Some(&authored.name),
                "advance",
                ParticleRefusalReason::ArithmeticOverflow,
            )
        };
        let mut index = 0;
        while index < self.live {
            let particle = &mut self.particles[index];
            particle.age = delta + particle.age;
            if !particle.age.is_finite() {
                return Err(fail());
            }
            if particle.age < 0.0 || particle.age > particle.life {
                self.live -= 1;
                if authored.trail {
                    // The tail remains a duplicate of the final survivor, including child identity.
                    self.particles.copy_within(index + 1..self.live + 1, index);
                } else {
                    self.swap_cpu_copy_gpu(self.live, index);
                }
            } else {
                index += 1;
            }
        }
        let child_count = authored.children.len();
        index = self.live;
        while index < self.retained {
            let first = self.particles[index].child_slot * child_count;
            if self.children[first..first + child_count]
                .iter()
                .any(|child| child.retained != 0)
            {
                index += 1;
            } else {
                self.retained -= 1;
                if authored.trail {
                    self.particles
                        .copy_within(index + 1..self.retained + 1, index);
                } else {
                    self.swap_cpu_copy_gpu(self.retained, index);
                }
            }
        }
        if self.end < 0.0 && self.pulse_duration > 0.0 && self.pulse_silence > 0.0 {
            self.pulse_time = delta + self.pulse_time;
            if !self.pulse_time.is_finite() {
                return Err(fail());
            }
            if self.pulse_on {
                if self.pulse_time > self.pulse_duration {
                    self.pulse_on = false;
                    self.pulse_time = 0.0;
                }
            } else if self.pulse_time > self.pulse_silence {
                self.local_time = delta;
                self.pulse_on = true;
                self.pulse_time = 0.0;
            }
        }
        if self.local_time >= 0.0
            && (self.local_time <= self.end || self.end < 0.0)
            && self.pulse_on
            && parent_alive
        {
            let emission = random.sample(&authored.emission, self.local_time);
            let emission = if emission < 0.0 { 0.0 } else { emission };
            self.fractional_emission = emission * delta + self.fractional_emission;
            if !self.fractional_emission.is_finite() || self.fractional_emission >= 2147483648.0 {
                return Err(fail());
            }
            if self.fractional_emission < 0.0 {
                self.fractional_emission = 0.0;
            }
            let amount = (self.fractional_emission as usize).min(authored.capacity - self.retained);
            // 1423e5cd0 returns before initializing an overlapping trail birth batch.
            // Its caller still advances both counts and consumes the emission fraction.
            if !authored.trail || amount.min(self.retained - self.live) == 0 {
                for index in 0..amount.min(self.retained - self.live) {
                    self.swap_cpu_copy_gpu(
                        self.live + index,
                        self.live + amount.max(self.retained - self.live) + index,
                    );
                }
                // Births occupy the live prefix; dead parents retain their child-slot identities.
                for index in self.live..self.live + amount {
                    let life = random.sample(&authored.life, self.local_time);
                    if !life.is_finite() {
                        return Err(fail());
                    }
                    self.particles[index].age = 0.0;
                    self.particles[index].life = life;
                    let first = self.particles[index].child_slot * child_count;
                    // Reuse resets child counts only; their clocks, sampled end and fraction persist.
                    for child in &mut self.children[first..first + child_count] {
                        child.live = 0;
                        child.retained = 0;
                    }
                }
                if amount > 0 {
                    for (column, property) in authored.spawn_draw_columns.iter().enumerate() {
                        for particle in &mut self.particles[self.live..self.live + amount] {
                            particle.visual.birth_columns[column] =
                                random.sample(property, self.local_time);
                        }
                    }
                    let emitter_angles = std::array::from_fn(|axis| {
                        random.sample(&authored.emitter_angles[axis], self.local_time)
                    });
                    for particle in &mut self.particles[self.live..self.live + amount] {
                        particle.visual.emitter_angles = emitter_angles;
                    }
                    for (column, property) in authored.velocity_angles_and_speed.iter().enumerate()
                    {
                        for particle in &mut self.particles[self.live..self.live + amount] {
                            particle.visual.velocity_angles_and_speed[column] =
                                random.sample(property, self.local_time);
                        }
                    }
                    if let Some(properties) = authored.shape.properties() {
                        for (column, property) in properties.iter().enumerate() {
                            for particle in &mut self.particles[self.live..self.live + amount] {
                                particle.visual.shape_coordinates[column] =
                                    random.sample(property, self.local_time);
                            }
                        }
                    }
                }
                for (index, particle) in self.particles[self.live..self.live + amount]
                    .iter_mut()
                    .enumerate()
                {
                    particle.visual.initialize();
                    particle.visual.initialize_motion(
                        authored,
                        &world,
                        self.previous_translation,
                        self.origin,
                        index,
                        amount,
                        scale,
                    )?;
                    particle.visual.atlas_draws = [random.draw(), random.draw()];
                }
            }
            self.live += amount;
            self.retained += amount;
            self.fractional_emission -= amount as f32;
        }
        self.previous_translation = translation;
        for particle in &mut self.particles[..self.live] {
            particle.visual.advance(
                authored,
                self.local_time,
                particle.age,
                particle.life,
                delta,
                opacity,
            );
        }
        if self.retained != 0 {
            self.advance_motion(authored, delta, scale, &world, random)?;
        }
        for index in 0..self.retained {
            let mut child_world = world;
            child_world[12..15].copy_from_slice(&self.particles[index].visual.position);
            child_world[15] = 1.0;
            let first = self.particles[index].child_slot * child_count;
            for (child, definition) in self.children[first..first + child_count]
                .iter_mut()
                .zip(&authored.children)
            {
                child.advance(
                    definition,
                    delta,
                    random,
                    index < self.live,
                    child_world,
                    instance_scale,
                    opacity,
                )?;
            }
        }
        self.local_time = delta + self.local_time;
        if !self.local_time.is_finite() {
            return Err(fail());
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Instance {
    handle: ParticleHandle,
    definition: Arc<AdmittedParticleDefinition>,
    random: ViewerParticleSeed,
    world: [u32; 16],
    speed: f32,
    opacity: f32,
    attached: bool,
    subsystems: Vec<Subsystem>,
    failure: Option<ParticleInputRefusal>,
    last_sort: Option<(u64, ParticleSortCamera)>,
}

impl Instance {
    fn effective_scale(&self) -> ParticleResult<f32> {
        let mut scale = self.definition.scale;
        if self
            .definition
            .subsystems
            .iter()
            .any(|system| system.local_space)
        {
            let world = self.world.map(f32::from_bits);
            let lengths = [0, 4, 8].map(|base| {
                world[base] * world[base]
                    + world[base + 1] * world[base + 1]
                    + world[base + 2] * world[base + 2]
            });
            let other = if lengths[1] <= lengths[2] {
                lengths[2]
            } else {
                lengths[1]
            };
            let largest = if lengths[0] <= other {
                other
            } else {
                lengths[0]
            };
            scale *= largest.sqrt();
        }
        if !scale.is_finite() {
            return Err(runtime_refusal(
                "particle_instance_scale",
                ParticleRefusalReason::ArithmeticOverflow,
            )
            .event(self.handle.event_instance_id));
        }
        Ok(scale)
    }
}

/// Call light_binding during entity-event preparation, then advance once in the
/// particle-update phase. 142276040 reads Finished before 1423ddaa0 advances time.
pub struct ParticleRuntime {
    identity: u64,
    generation: u64,
    lease: ContentAdmissionLease,
    instances: Vec<Option<Instance>>,
    touched: Vec<bool>,
    presentation: Option<u64>,
}

impl ParticleRuntime {
    pub fn new(lease: ContentAdmissionLease, instance_capacity: usize) -> ParticleResult<Self> {
        let mut instances = Vec::new();
        let mut touched = Vec::new();
        instances
            .try_reserve_exact(instance_capacity)
            .map_err(|_| {
                runtime_refusal("instance_capacity", ParticleRefusalReason::CapacityExceeded)
            })?;
        touched.try_reserve_exact(instance_capacity).map_err(|_| {
            runtime_refusal("instance_capacity", ParticleRefusalReason::CapacityExceeded)
        })?;
        instances.resize_with(instance_capacity, || None);
        touched.resize(instance_capacity, false);
        let identity = NEXT_RUNTIME
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| {
                runtime_refusal("runtime_identity", ParticleRefusalReason::CapacityExceeded)
            })?;
        Ok(Self {
            identity,
            generation: 0,
            lease,
            instances,
            touched,
            presentation: None,
        })
    }

    pub fn spawn(
        &mut self,
        event_instance_id: u64,
        definition: Arc<AdmittedParticleDefinition>,
        mut viewer_seed: ViewerParticleSeed,
        parent_world_matrix_f32_bits: [u32; 16],
        inherited_speed: f32,
    ) -> ParticleResult<ParticleHandle> {
        let fail = |input: &str, reason| {
            ParticleInputRefusal::new(definition.name(), None, input, reason)
                .event(event_instance_id)
        };
        if definition.lease != self.lease {
            return Err(fail(
                "content_session",
                ParticleRefusalReason::WrongContentSession,
            ));
        }
        if !valid_matrix(parent_world_matrix_f32_bits) || !inherited_speed.is_finite() {
            return Err(fail(
                "spawn_transform_or_speed",
                ParticleRefusalReason::InvalidNumber,
            ));
        }
        if self
            .instances
            .iter()
            .flatten()
            .any(|i| i.handle.event_instance_id == event_instance_id)
        {
            return Err(fail(
                "event_instance_id",
                ParticleRefusalReason::DuplicateEventInstance,
            ));
        }
        let slot = self
            .instances
            .iter()
            .position(Option::is_none)
            .ok_or_else(|| fail("instance_capacity", ParticleRefusalReason::CapacityExceeded))?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| fail("generation", ParticleRefusalReason::CapacityExceeded))?;
        let mut subsystems = Vec::new();
        subsystems
            .try_reserve_exact(definition.subsystems.len())
            .map_err(|_| fail("subsystems", ParticleRefusalReason::CapacityExceeded))?;
        for authored in definition.subsystems.iter() {
            let subsystem = Subsystem::new(authored, &mut viewer_seed).map_err(|mut e| {
                e.definition = definition.name().into();
                e.event(event_instance_id)
            })?;
            subsystems.push(subsystem);
        }
        for _ in 0..definition.turbulence_count {
            viewer_seed.initialize_turbulence();
        }
        let handle = ParticleHandle {
            runtime: self.identity,
            generation,
            slot,
            event_instance_id,
        };
        self.generation = generation;
        self.instances[slot] = Some(Instance {
            handle,
            definition,
            random: viewer_seed,
            world: parent_world_matrix_f32_bits,
            speed: inherited_speed,
            opacity: 1.0,
            attached: true,
            subsystems,
            failure: None,
            last_sort: None,
        });
        Ok(handle)
    }

    pub fn sort_for_draw(
        &mut self,
        handle: ParticleHandle,
        presentation_id: u64,
        camera: ParticleSortCamera,
    ) -> ParticleResult<()> {
        let instance = self.instance(handle)?;
        if let Some(error) = &instance.failure {
            return Err(error.clone());
        }
        if self.presentation != Some(presentation_id) {
            return Err(runtime_refusal(
                "sort_presentation_id",
                ParticleRefusalReason::PresentationOutOfOrder,
            )
            .event(handle.event_instance_id));
        }
        if let Some((prior, previous_camera)) = instance.last_sort {
            if prior == presentation_id {
                return if camera == previous_camera {
                    Ok(())
                } else {
                    Err(runtime_refusal(
                        "sort_camera",
                        ParticleRefusalReason::DuplicatePresentation,
                    )
                    .event(handle.event_instance_id))
                };
            }
        }
        if camera
            .position_f32_bits
            .iter()
            .chain(camera.forward_f32_bits.iter())
            .any(|bits| !f32::from_bits(*bits).is_finite())
        {
            return Err(
                runtime_refusal("sort_camera", ParticleRefusalReason::InvalidNumber)
                    .event(handle.event_instance_id),
            );
        }
        let instance = self.instances[handle.slot].as_mut().unwrap();
        if instance.attached {
            for (state, authored) in instance
                .subsystems
                .iter_mut()
                .zip(instance.definition.subsystems.iter())
            {
                if let Err(mut error) = state.sort_for_draw(authored, camera) {
                    error.definition = instance.definition.name().into();
                    error.event_instance_id = Some(handle.event_instance_id);
                    instance.failure = Some(error.clone());
                    return Err(error);
                }
            }
        }
        instance.last_sort = Some((presentation_id, camera));
        Ok(())
    }

    pub fn gpu_view(&self, handle: ParticleHandle) -> ParticleResult<ParticleGpuView<'_>> {
        let instance = self.instance(handle)?;
        if let Some(refusal) = &instance.failure {
            return Err(refusal.clone());
        }
        Ok(ParticleGpuView {
            instance,
            presentation_id: self.presentation,
        })
    }

    pub fn set_opacity(&mut self, handle: ParticleHandle, opacity: f32) -> ParticleResult<()> {
        self.instance(handle)?;
        if !opacity.is_finite() {
            return Err(
                runtime_refusal("opacity", ParticleRefusalReason::InvalidNumber)
                    .event(handle.event_instance_id),
            );
        }
        self.instances[handle.slot].as_mut().unwrap().opacity = opacity;
        Ok(())
    }

    pub fn light_binding(&self, handle: ParticleHandle) -> ParticleResult<ParticleLightBinding> {
        let instance = self.instance(handle)?;
        if let Some(refusal) = &instance.failure {
            return Err(refusal.clone());
        }
        Ok(ParticleLightBinding {
            event_instance_id: handle.event_instance_id,
            presentation_id: self.presentation,
            world_matrix_f32_bits: instance.world,
            finished: instance.subsystems.iter().all(Subsystem::finished),
        })
    }

    pub fn refresh_attachment(
        &mut self,
        handle: ParticleHandle,
        matrix: [u32; 16],
        speed: f32,
    ) -> ParticleResult<()> {
        self.instance(handle)?;
        if !valid_matrix(matrix) || !speed.is_finite() {
            return Err(
                runtime_refusal("attachment_update", ParticleRefusalReason::InvalidNumber)
                    .event(handle.event_instance_id),
            );
        }
        let instance = self.instances[handle.slot].as_mut().unwrap();
        if instance.attached {
            instance.world = matrix;
        }
        instance.speed = speed;
        Ok(())
    }

    pub fn advance(
        &mut self,
        presentation_id: u64,
        renderer_delta: f32,
        updates: &[ParticleInstanceUpdate],
    ) -> ParticleResult<()> {
        if self.presentation == Some(presentation_id) {
            return Err(runtime_refusal(
                "presentation_id",
                ParticleRefusalReason::DuplicatePresentation,
            ));
        }
        if self
            .presentation
            .is_some_and(|previous| previous > presentation_id)
        {
            return Err(runtime_refusal(
                "presentation_id",
                ParticleRefusalReason::PresentationOutOfOrder,
            ));
        }
        if !renderer_delta.is_finite() || renderer_delta < 0.0 {
            return Err(runtime_refusal(
                "renderer_delta",
                ParticleRefusalReason::InvalidNumber,
            ));
        }
        self.touched.fill(false);
        for update in updates {
            self.instance(update.handle)?;
            if self.touched[update.handle.slot] {
                return Err(runtime_refusal(
                    "notification",
                    ParticleRefusalReason::DuplicateNotification,
                )
                .event(update.handle.event_instance_id));
            }
            self.touched[update.handle.slot] = true;
            if update
                .parent_world_matrix_f32_bits
                .is_some_and(|matrix| !valid_matrix(matrix))
                || update
                    .inherited_speed
                    .is_some_and(|speed| !speed.is_finite())
            {
                return Err(runtime_refusal(
                    "attachment_update",
                    ParticleRefusalReason::InvalidNumber,
                )
                .event(update.handle.event_instance_id));
            }
        }
        for update in updates {
            if update.action == ParticleAction::Destroy {
                self.instances[update.handle.slot] = None;
                continue;
            }
            let instance = self.instances[update.handle.slot].as_mut().unwrap();
            if instance.attached {
                if let Some(matrix) = update.parent_world_matrix_f32_bits {
                    instance.world = matrix;
                }
            }
            if let Some(speed) = update.inherited_speed {
                instance.speed = speed;
            }
            match update.action {
                ParticleAction::StopUnboundedEmission => {
                    if instance.subsystems.iter().any(|s| s.end < 0.0) {
                        for subsystem in &mut instance.subsystems {
                            subsystem.end = subsystem.local_time;
                        }
                    }
                }
                ParticleAction::StopEmission | ParticleAction::StopAndDetach => {
                    for subsystem in &mut instance.subsystems {
                        subsystem.end = subsystem.local_time;
                    }
                    if update.action == ParticleAction::StopAndDetach {
                        instance.attached = false;
                    }
                }
                ParticleAction::Detach => instance.attached = false,
                ParticleAction::Continue | ParticleAction::Destroy => {}
            }
        }
        self.presentation = Some(presentation_id);
        let mut first_failure = None;
        for instance in self.instances.iter_mut().flatten() {
            if instance.failure.is_some() {
                continue;
            }
            let delta = instance.speed * renderer_delta;
            let mut failure = if delta.is_finite() {
                None
            } else {
                Some(runtime_refusal(
                    "scaled_renderer_delta",
                    ParticleRefusalReason::ArithmeticOverflow,
                ))
            };
            let scale = instance.effective_scale();
            if let Err(error) = &scale {
                if failure.is_none() {
                    failure = Some(error.clone());
                }
            }
            if failure.is_none() {
                let scale = scale.unwrap();
                for (state, authored) in instance
                    .subsystems
                    .iter_mut()
                    .zip(instance.definition.subsystems.iter())
                {
                    if let Err(error) = state.advance(
                        authored,
                        delta,
                        &mut instance.random,
                        true,
                        instance.world.map(f32::from_bits),
                        scale,
                        instance.opacity,
                    ) {
                        failure = Some(error);
                        break;
                    }
                }
            }
            if let Some(mut error) = failure {
                error.definition = instance.definition.name().into();
                error.event_instance_id = Some(instance.handle.event_instance_id);
                if first_failure.is_none() {
                    first_failure = Some(error.clone());
                }
                instance.failure = Some(error);
            }
        }
        match first_failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn reset(&mut self) {
        for slot in &mut self.instances {
            *slot = None;
        }
        self.presentation = None;
    }

    pub fn live_instance_count(&self) -> usize {
        self.instances.iter().flatten().count()
    }

    pub fn allocated_particle_capacity(&self) -> usize {
        self.instances
            .iter()
            .flatten()
            .flat_map(|i| &i.subsystems)
            .map(Subsystem::allocated_capacity)
            .sum()
    }

    fn instance(&self, handle: ParticleHandle) -> ParticleResult<&Instance> {
        self.instances
            .get(handle.slot)
            .and_then(Option::as_ref)
            .filter(|instance| handle.runtime == self.identity && instance.handle == handle)
            .ok_or_else(|| {
                runtime_refusal("handle", ParticleRefusalReason::UnknownHandle)
                    .event(handle.event_instance_id)
            })
    }
}

fn runtime_refusal(input: &str, reason: ParticleRefusalReason) -> ParticleInputRefusal {
    ParticleInputRefusal::new("", None, input, reason)
}

fn valid_matrix(matrix: [u32; 16]) -> bool {
    matrix
        .into_iter()
        .all(|bits| f32::from_bits(bits).is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle_authored::{
        test_support::{Fixture, REGISTRATION},
        ParticleDefinitions,
    };

    fn matrix() -> [u32; 16] {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 7.0, -0.0, 11.0, 1.0,
        ]
        .map(f32::to_bits)
    }
    fn setup(body: &str) -> (Fixture, ParticleRuntime, ParticleHandle) {
        let fixture = Fixture::new(REGISTRATION, &format!("particle={{name=effect {body}}}"));
        let mut definitions = fixture.definitions();
        let mut runtime = ParticleRuntime::new(definitions.content_lease(), 2).unwrap();
        let handle = runtime
            .spawn(
                17,
                definitions.resolve("paired").unwrap(),
                ViewerParticleSeed {
                    counter: 0,
                    seed: 29,
                },
                matrix(),
                1.0,
            )
            .unwrap();
        (fixture, runtime, handle)
    }

    fn gpu_records(runtime: &ParticleRuntime, handle: ParticleHandle) -> Vec<ParticleGpuRecord> {
        runtime
            .gpu_view(handle)
            .unwrap()
            .root_subsystems()
            .next()
            .unwrap()
            .records()
            .collect::<ParticleResult<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn draw_root_matrix_normalizes_each_basis_and_preserves_zero_axis_and_translation() {
        for local in [false, true] {
            let (_fixture, mut runtime, handle) = setup(&format!(
                "subsystem={{name=s max_amount=0 local_space={}}}",
                if local { "yes" } else { "no" }
            ));
            let world = [
                3.0_f32, 4.0, 0.0, 10.0, 0.0, -2.0, 0.0, 6.0, -0.0, 0.0, 0.0, -0.0, 7.0, -0.0,
                11.0, 1.0,
            ];
            runtime.instances[handle.slot].as_mut().unwrap().world = world.map(f32::to_bits);
            let actual = runtime
                .gpu_view(handle)
                .unwrap()
                .root_subsystems()
                .next()
                .unwrap()
                .draw_world_matrix_f32_bits()
                .unwrap();
            let expected = if local {
                [
                    3.0 * 0.2,
                    4.0 * 0.2,
                    0.0,
                    2.0,
                    0.0,
                    -1.0,
                    0.0,
                    3.0,
                    -0.0,
                    0.0,
                    0.0,
                    -0.0,
                    7.0,
                    -0.0,
                    11.0,
                    1.0,
                ]
            } else {
                [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ]
            };
            assert_eq!(actual, expected.map(f32::to_bits));
            assert_eq!(
                runtime.light_binding(handle).unwrap().world_matrix_f32_bits,
                world.map(f32::to_bits)
            );
        }
    }

    #[test]
    fn draw_matrix_overflow_is_named_without_changing_the_paired_light_matrix() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=0 local_space=yes}");
        let mut world = matrix().map(f32::from_bits);
        world[..4].copy_from_slice(&[3.0e-20, 0.0, 0.0, 1.0e20]);
        runtime.instances[handle.slot].as_mut().unwrap().world = world.map(f32::to_bits);
        let error = runtime
            .gpu_view(handle)
            .unwrap()
            .root_subsystems()
            .next()
            .unwrap()
            .draw_world_matrix_f32_bits()
            .unwrap_err();
        assert_eq!(error.input, "particle_draw_matrix");
        assert_eq!(error.reason, ParticleRefusalReason::ArithmeticOverflow);
        assert_eq!(
            runtime.light_binding(handle).unwrap().world_matrix_f32_bits,
            world.map(f32::to_bits)
        );
    }

    #[test]
    fn draw_visits_retained_children_in_each_native_trail_partition() {
        let (_fixture, mut runtime, handle) = setup(
            "subsystem={name=root max_amount=2 emission=8 life=5 trail=yes childsystem={name=child max_amount=0 local_space=yes}} subsystem={name=last max_amount=0}");
        runtime.advance(1, 0.25, &[]).unwrap();
        let state = &mut runtime.instances[handle.slot].as_mut().unwrap().subsystems[0];
        state.live = 0;
        state.particles[0].visual.position = [1.0, -0.0, 3.0];
        state.particles[1].visual.position = [4.0, 5.0, 6.0];
        let view = runtime.gpu_view(handle).unwrap();
        let mut normal = Vec::new();
        view.visit_draw_subsystems(4, false, |_, system| {
            normal.push((system.name(), system.draw_world_matrix_f32_bits()?));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            normal.iter().map(|row| row.0).collect::<Vec<_>>(),
            ["child", "child", "last"]
        );
        for (row, position) in normal[..2]
            .iter()
            .zip([[1.0_f32, -0.0, 3.0], [4.0, 5.0, 6.0]])
        {
            let mut expected = [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ];
            expected[12..15].copy_from_slice(&position);
            assert_eq!(row.1, expected.map(f32::to_bits));
        }
        let mut trail = Vec::new();
        view.visit_draw_subsystems(4, true, |_, system| {
            trail.push((system.name(), system.live_count()));
            Ok(())
        })
        .unwrap();
        assert_eq!(trail, [("root", 0)]);
    }

    #[test]
    fn particle_draw_scale_uses_largest_attachment_basis_only_for_local_roots() {
        for (body, expected) in [
            ("subsystem={name=s max_amount=0}", [2.0, 1.0]),
            (
                "subsystem={name=s max_amount=0 local_space=yes}",
                [20.0, 20.0],
            ),
            (
                "subsystem={name=s max_amount=0 childsystem={name=c max_amount=1 local_space=yes}}",
                [2.0, 1.0],
            ),
            (
                "subsystem={name=s max_amount=0} subsystem={name=t max_amount=0 local_space=yes}",
                [20.0, 20.0],
            ),
        ] {
            let (_fixture, mut runtime, handle) = setup(body);
            let mut world = matrix().map(f32::from_bits);
            world[..3].copy_from_slice(&[3.0, 4.0, 0.0]);
            world[4..7].copy_from_slice(&[0.0, 8.0, 6.0]);
            world[8..11].copy_from_slice(&[0.0, 0.0, -2.0]);
            runtime
                .refresh_attachment(handle, world.map(f32::to_bits), 1.0)
                .unwrap();
            assert_eq!(
                runtime
                    .gpu_view(handle)
                    .unwrap()
                    .draw_scale_f32_bits()
                    .unwrap(),
                expected.map(f32::to_bits)
            );
            runtime.advance(1, 0.0, &[]).unwrap();
            assert_eq!(
                runtime.light_binding(handle).unwrap().world_matrix_f32_bits,
                world.map(f32::to_bits)
            );
        }
    }

    #[test]
    fn particle_scale_overflow_refuses_public_advance_and_remains_named() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=1 emission=2 life=1 local_space=yes}");
        let mut world = matrix();
        world[0] = f32::MAX.to_bits();
        runtime.refresh_attachment(handle, world, 1.0).unwrap();
        let failure = runtime.advance(1, 0.5, &[]).unwrap_err();
        assert_eq!(failure.input, "particle_instance_scale");
        assert_eq!(failure.reason, ParticleRefusalReason::ArithmeticOverflow);
        assert_eq!(runtime.gpu_view(handle).unwrap_err(), failure);
        runtime.advance(2, 0.5, &[]).unwrap();
        assert_eq!(runtime.gpu_view(handle).unwrap_err(), failure);
    }

    #[test]
    fn draw_capacity_counts_both_partitions_and_zero_live_retained_children() {
        let (_fixture, mut runtime, handle) = setup(
            "subsystem={name=root max_amount=2 emission=8 life=5 trail=yes childsystem={name=child max_amount=0}} subsystem={name=last max_amount=0}");
        runtime.advance(1, 0.25, &[]).unwrap();
        runtime.instances[handle.slot].as_mut().unwrap().subsystems[0].live = 0;
        let view = runtime.gpu_view(handle).unwrap();
        for capacity in 0..=5 {
            let mut normal = Vec::new();
            let count = view
                .visit_draw_subsystems(capacity, false, |ordinal, system| {
                    normal.push((ordinal, system.name(), system.live_count()));
                    Ok(())
                })
                .unwrap();
            assert_eq!(count, capacity.min(4));
            let expected = [(1, "child", 0), (2, "child", 0), (3, "last", 0)];
            assert_eq!(
                normal,
                expected
                    .into_iter()
                    .filter(|row| row.0 < capacity)
                    .collect::<Vec<_>>()
            );
            let mut trail = Vec::new();
            assert_eq!(
                view.visit_draw_subsystems(capacity, true, |ordinal, system| {
                    trail.push((ordinal, system.name(), system.live_count()));
                    Ok(())
                })
                .unwrap(),
                count
            );
            assert_eq!(
                trail,
                if capacity == 0 {
                    vec![]
                } else {
                    vec![(0, "root", 0)]
                }
            );
        }
        let error = view
            .visit_draw_subsystems(i32::MAX as usize + 1, false, |_, _| {
                panic!("invalid native capacity must not visit")
            })
            .unwrap_err();
        let error = error.downcast::<ParticleInputRefusal>().unwrap();
        assert_eq!(error.input, "particle_draw_packet_capacity");
        assert_eq!(error.reason, ParticleRefusalReason::InvalidNumber);
        assert_eq!(error.event_instance_id, Some(handle.event_instance_id));
    }

    #[test]
    fn trail_vertices_keep_native_pair_layout_and_fixed_plane_join() {
        for billboard in [false, true] {
            let (_fixture, mut runtime, handle) = setup(&format!(
                "subsystem={{name=s max_amount=3 duration=-1 emission=12 life=5 size=1 trail=yes billboard={}}}",
                if billboard { "yes" } else { "no" }));
            runtime.advance(1, 0.25, &[]).unwrap();
            let system = &mut runtime.instances[handle.slot].as_mut().unwrap().subsystems[0];
            assert_eq!(system.live, 3);
            for (particle, position) in
                system
                    .particles
                    .iter_mut()
                    .zip([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0]])
            {
                particle.visual.position = position;
                particle.visual.gpu_color = [1, 2, 3, 4];
            }
            let view = runtime
                .gpu_view(handle)
                .unwrap()
                .root_subsystems()
                .next()
                .unwrap();
            let mut vertices = Vec::new();
            view.visit_trail_vertices([0, 0, 4.0_f32.to_bits()], 2.0_f32.to_bits(), |bytes| {
                vertices.push(bytes);
                Ok(())
            })
            .unwrap();
            assert_eq!(vertices.len(), 6);
            let positions: Vec<[f32; 3]> = vertices
                .iter()
                .map(|bytes| {
                    std::array::from_fn(|axis| {
                        f32::from_le_bytes(bytes[axis * 4..axis * 4 + 4].try_into().unwrap())
                    })
                })
                .collect();
            if billboard {
                assert_eq!(
                    positions,
                    [
                        [0.0, 2.0, 0.0],
                        [0.0, -2.0, 0.0],
                        [2.0, 2.0, 0.0],
                        [2.0, -2.0, 0.0],
                        [2.0, 2.0, 2.0],
                        [2.0, -2.0, 2.0]
                    ]
                );
            } else {
                let diagonal = f32::from_bits(0x3f3504f3);
                assert_eq!(
                    positions,
                    [
                        [0.0, 0.0, 2.0],
                        [0.0, 0.0, -2.0],
                        [(1.0 - diagonal) * 2.0, 0.0, diagonal * 2.0],
                        [(1.0 + diagonal) * 2.0, 0.0, -diagonal * 2.0],
                        [(1.0 - diagonal) * 2.0, 0.0, (1.0 + diagonal) * 2.0],
                        [(1.0 + diagonal) * 2.0, 0.0, (1.0 - diagonal) * 2.0]
                    ]
                );
            }
            for (index, bytes) in vertices.iter().enumerate() {
                assert_eq!(&bytes[12..16], &((index / 2) as f32).to_le_bytes());
                assert_eq!(
                    &bytes[16..20],
                    &(if index % 2 == 0 { 1.0_f32 } else { 0.0 }).to_le_bytes()
                );
                assert_eq!(&bytes[20..24], &[0; 4]);
                assert_eq!(&bytes[24..28], &[1, 2, 3, 4]);
            }
        }
    }

    #[test]
    fn trail_degenerate_segments_and_single_slots_follow_native_draw_count() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=2 duration=-1 emission=8 life=5 size=1 trail=yes}");
        runtime.advance(1, 0.25, &[]).unwrap();
        for count in [0, 1, 2] {
            let system = &mut runtime.instances[handle.slot].as_mut().unwrap().subsystems[0];
            system.live = count;
            for particle in &mut system.particles {
                particle.visual.position = [3.0, 4.0, 5.0];
            }
            let view = runtime
                .gpu_view(handle)
                .unwrap()
                .root_subsystems()
                .next()
                .unwrap();
            let mut vertices = Vec::new();
            view.visit_trail_vertices([0; 3], 1.0_f32.to_bits(), |bytes| {
                vertices.push(bytes);
                Ok(())
            })
            .unwrap();
            assert_eq!(vertices.len(), if count == 2 { 4 } else { 0 });
            for bytes in vertices {
                let actual: [f32; 3] = std::array::from_fn(|axis| {
                    f32::from_le_bytes(bytes[axis * 4..axis * 4 + 4].try_into().unwrap())
                });
                assert_eq!(actual, [3.0, 4.0, 5.0]);
            }
        }
        let system = &mut runtime.instances[handle.slot].as_mut().unwrap().subsystems[0];
        system.particles[1].visual.position = [4.0, 4.0, 5.0];
        let view = runtime
            .gpu_view(handle)
            .unwrap()
            .root_subsystems()
            .next()
            .unwrap();
        let error = view
            .visit_trail_vertices([0; 3], f32::MAX.to_bits(), |_| Ok(()))
            .unwrap_err();
        assert_eq!(error.input, "trail_vertex_position");
        assert_eq!(error.reason, ParticleRefusalReason::ArithmeticOverflow);
    }

    #[test]
    fn draw_requires_sort_for_the_current_presentation() {
        let (_fixture, mut runtime, handle) = setup("subsystem={name=s max_amount=0}");
        runtime.advance(1, 0.25, &[]).unwrap();
        let camera = ParticleSortCamera {
            position_f32_bits: [0; 3],
            forward_f32_bits: [0, 0, 1.0_f32.to_bits()],
        };
        let check = |runtime: &ParticleRuntime, id| {
            runtime
                .gpu_view(handle)
                .unwrap()
                .root_subsystems()
                .next()
                .unwrap()
                .validate_draw_presentation(id, camera)
        };
        assert_eq!(
            check(&runtime, 1).unwrap_err().input,
            "particle_draw_presentation"
        );
        runtime.sort_for_draw(handle, 1, camera).unwrap();
        check(&runtime, 1).unwrap();
        let other_camera = ParticleSortCamera {
            position_f32_bits: [1.0_f32.to_bits(), 0, 0],
            ..camera
        };
        assert!(runtime
            .gpu_view(handle)
            .unwrap()
            .root_subsystems()
            .next()
            .unwrap()
            .validate_draw_presentation(1, other_camera)
            .is_err());
        assert!(check(&runtime, 2).is_err());
        runtime.advance(2, 0.25, &[]).unwrap();
        assert!(check(&runtime, 1).is_err());
        assert!(check(&runtime, 2).is_err());
        runtime.sort_for_draw(handle, 2, camera).unwrap();
        check(&runtime, 2).unwrap();
    }

    #[test]
    fn sort_orders_match_native_cpu_and_gpu_instruction_fixtures() {
        let data: serde_json::Value =
            serde_json::from_str(portable_engine_evidence::particle_runtime::SORT_CASES).unwrap();
        let read_vector =
            |value: &serde_json::Value| std::array::from_fn(|i| value[i].as_u64().unwrap() as u32);
        let camera = ParticleSortCamera {
            position_f32_bits: read_vector(&data["camera_position_f32_bits"]),
            forward_f32_bits: read_vector(&data["camera_forward_f32_bits"]),
        };
        let mut calls = 0;
        for case in data["cases"].as_array().unwrap() {
            let sort = [
                ParticleSort::None,
                ParticleSort::Depth,
                ParticleSort::Distance,
                ParticleSort::Age,
            ][case["sort"].as_u64().unwrap() as usize];
            let (_fixture, mut runtime, handle) =
                setup("subsystem={name=s max_amount=129 duration=0 emission=0 life=4}");
            let instance = runtime.instances[handle.slot].as_mut().unwrap();
            let mut authored = instance.definition.subsystems[0].clone();
            authored.sort = sort;
            authored.invert = case["invert"].as_bool().unwrap();
            let state = &mut instance.subsystems[0];
            let slots = case["slots"].as_array().unwrap();
            state.live = slots.len();
            state.retained = slots.len();
            for (index, (particle, input)) in state.particles.iter_mut().zip(slots).enumerate() {
                particle.age = f32::from_bits(input["age_f32_bits"].as_u64().unwrap() as u32);
                particle.visual.position =
                    read_vector(&input["position_f32_bits"]).map(f32::from_bits);
                particle.visual.atlas_draws = [index as u32, 0];
            }
            assert_eq!(state.sort_phase, 1);
            for (frame, expected) in case["frames"].as_array().unwrap().iter().enumerate() {
                state.sort_for_draw(&authored, camera).unwrap();
                let cpu_order: Vec<usize> = state.particles[..state.live]
                    .iter()
                    .map(|p| p.child_slot)
                    .collect();
                let gpu_order: Vec<u32> = state.particles[..state.live]
                    .iter()
                    .map(|p| p.visual.atlas_draws[0])
                    .collect();
                assert_eq!(
                    serde_json::json!(cpu_order),
                    expected["cpu_order"],
                    "{sort:?} / {} / {} / {frame}",
                    authored.invert,
                    slots.len()
                );
                assert_eq!(serde_json::json!(gpu_order), expected["gpu_order"]);
                assert_eq!(state.sort_phase as u64, expected["phase"].as_u64().unwrap());
                calls += 1;
            }
        }
        assert_eq!(calls, 1344);
    }

    #[test]
    fn public_sort_runs_once_per_presentation_and_preserves_native_detach_gate() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=9 duration=0 emission=36 life=4 sort=age}");
        let camera = ParticleSortCamera {
            position_f32_bits: [0; 3],
            forward_f32_bits: [0, 0, 1.0_f32.to_bits()],
        };
        assert_eq!(
            runtime.sort_for_draw(handle, 1, camera).unwrap_err().reason,
            ParticleRefusalReason::PresentationOutOfOrder
        );
        runtime.advance(1, 0.25, &[]).unwrap();
        runtime.sort_for_draw(handle, 1, camera).unwrap();
        assert_eq!(
            runtime.instance(handle).unwrap().subsystems[0].sort_phase,
            2
        );
        runtime.sort_for_draw(handle, 1, camera).unwrap();
        assert_eq!(
            runtime.instance(handle).unwrap().subsystems[0].sort_phase,
            2
        );
        assert_eq!(
            runtime.gpu_view(handle).unwrap().sorted_presentation_id(),
            Some(1)
        );
        let other = ParticleSortCamera {
            position_f32_bits: [1.0_f32.to_bits(); 3],
            ..camera
        };
        assert_eq!(
            runtime.sort_for_draw(handle, 1, other).unwrap_err().reason,
            ParticleRefusalReason::DuplicatePresentation
        );
        runtime
            .advance(
                2,
                0.25,
                &[ParticleInstanceUpdate {
                    handle,
                    parent_world_matrix_f32_bits: None,
                    inherited_speed: None,
                    action: ParticleAction::Detach,
                }],
            )
            .unwrap();
        runtime.sort_for_draw(handle, 2, camera).unwrap();
        assert_eq!(
            runtime.instance(handle).unwrap().subsystems[0].sort_phase,
            2
        );
    }

    #[test]
    fn gpu_records_keep_native_layout_atlas_color_and_frame_identity() {
        let (_fixture, mut runtime, handle) = setup("subsystem={name=s max_amount=1 duration=0 emission=4 life=4 size=2 particle_roll=3 rotation_speed=4 color={r=300 g=-2 b=10 a=129} texture={file=unused.dds x=7 y=-9}}");
        runtime.set_opacity(handle, 0.5).unwrap();
        runtime.advance(1, 0.25, &[]).unwrap();
        let view = runtime.gpu_view(handle).unwrap();
        assert_eq!(view.handle(), handle);
        assert_eq!(view.presentation_id, Some(1));
        assert_eq!(view.content_lease(), &runtime.lease);
        assert_eq!(view.definition().registration(), "paired");
        assert_eq!(view.world_matrix_f32_bits(), matrix());
        let subsystem = view.root_subsystems().next().unwrap();
        assert_eq!(subsystem.authored_capacity(), 1);
        assert_eq!(subsystem.live_count(), 1);
        let record = subsystem.records().next().unwrap().unwrap().0;
        let fields: Vec<u32> = record[..28]
            .chunks_exact(4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
            .collect();
        assert_eq!(
            fields,
            [7.0_f32, 0.0, 11.0, 2.0, 0.0, 0.0, 4.0].map(f32::to_bits)
        );
        let draws = runtime.instance(handle).unwrap().subsystems[0].particles[0]
            .visual
            .atlas_draws;
        assert_eq!(
            &record[28..32],
            &[(draws[0] % 7) as u8, (draws[1] % 9) as u8, 0, 0]
        );
        assert_eq!(&record[32..36], &[10, 0, 255, 65]);
        assert_eq!(&record[36..], &[0; 12]);
        runtime.advance(2, 0.25, &[]).unwrap();
        assert_eq!(gpu_records(&runtime, handle)[0].0[35], 33);
        runtime.set_opacity(handle, 2.0).unwrap();
        runtime.advance(3, 0.25, &[]).unwrap();
        assert_eq!(gpu_records(&runtime, handle)[0].0[35], 33);
        assert_eq!(runtime.gpu_view(handle).unwrap().presentation_id, Some(3));
    }

    #[test]
    fn gpu_color_animation_resets_alpha_before_each_opacity_application() {
        let (_fixture, mut runtime, handle) = setup("animation={name=alpha time=life_abs minValue=1 maxValue=1 curve={0 0 1 1}} subsystem={name=s max_amount=1 duration=0 emission=4 life=4 color={a=128,alpha}}");
        runtime.set_opacity(handle, 0.5).unwrap();
        for frame in 1..=3 {
            runtime.advance(frame, 0.25, &[]).unwrap();
            assert_eq!(gpu_records(&runtime, handle)[0].0[35], 65);
        }
    }

    #[test]
    fn gpu_export_refuses_invalid_atlas_without_refusing_unused_light_resource() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=1 duration=0 emission=4 life=4 texture={x=0}}");
        runtime.advance(1, 0.25, &[]).unwrap();
        assert!(!runtime.light_binding(handle).unwrap().finished);
        let error = runtime
            .gpu_view(handle)
            .unwrap()
            .root_subsystems()
            .next()
            .unwrap()
            .records()
            .next()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.input, "texture_atlas_dimension");
        assert_eq!(error.event_instance_id, Some(17));
        assert_eq!(error.reason, ParticleRefusalReason::ArithmeticOverflow);
        assert_eq!(
            runtime.set_opacity(handle, f32::NAN).unwrap_err().reason,
            ParticleRefusalReason::InvalidNumber
        );
        assert_eq!(runtime.instance(handle).unwrap().opacity, 1.0);
    }

    #[test]
    fn gpu_export_checks_stored_motion_failure_and_draw_only_nonfinite_values() {
        let (_fixture, mut runtime, handle) = setup("force={name=f amount=1} subsystem={name=s max_amount=1 duration=0 emission=4 life=4 force=f}");
        let error = runtime.advance(1, 0.25, &[]).unwrap_err();
        assert_eq!(runtime.gpu_view(handle).unwrap_err(), error);
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=1 duration=0 emission=4 life=4}");
        runtime.advance(1, 0.25, &[]).unwrap();
        runtime.instances[handle.slot].as_mut().unwrap().subsystems[0].particles[0]
            .visual
            .columns[0] = f32::INFINITY;
        let error = runtime
            .gpu_view(handle)
            .unwrap()
            .root_subsystems()
            .next()
            .unwrap()
            .records()
            .next()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.input, "gpu_position_size_rotation");
        assert_eq!(error.reason, ParticleRefusalReason::ArithmeticOverflow);
    }

    #[test]
    fn public_advance_uses_native_signed_birth_angles_outside_the_curve_range() {
        let (_fixture, mut runtime, handle) = setup("subsystem={name=s max_amount=1 duration=0 emission=4 life=4 emitter_yaw=270 velocity=1}");
        runtime.advance(1, 0.25, &[]).unwrap();
        let visual =
            runtime.instances[handle.slot].as_ref().unwrap().subsystems[0].particles[0].visual;
        assert_eq!(
            visual.velocity.map(f32::to_bits),
            [0x40000000, 0x34bbbd2e, 0]
        );
        assert_eq!(
            visual.position.map(f32::to_bits),
            [8.0f32.to_bits(), 0x343bbd2e, 11.0f32.to_bits()]
        );
        assert_eq!(
            runtime.light_binding(handle).unwrap().presentation_id,
            Some(1)
        );
    }

    #[test]
    fn public_advance_refuses_unsupported_motion_and_keeps_the_instance_failure() {
        for (kind, extra) in [("turbulence", ""), ("spin", "")] {
            let (_fixture, mut runtime, handle) = setup(&format!("force={{name=unclosed type={kind} amount=1 {extra}}} subsystem={{name=s max_amount=1 duration=0 emission=4 life=4 mass=1 force=unclosed}}"));
            let error = runtime.advance(1, 0.25, &[]).unwrap_err();
            assert_eq!(error.event_instance_id, Some(17));
            assert_eq!(error.definition, "effect");
            assert_eq!(error.subsystem.as_deref(), Some("s"));
            assert_eq!(
                error.reason,
                ParticleRefusalReason::UnsupportedAuthoredInput
            );
            assert!(error.input.starts_with("force:unclosed:"));
            assert_eq!(runtime.light_binding(handle).unwrap_err(), error);
        }
    }

    #[test]
    fn public_advance_refuses_nonfinite_zero_mass_force_result_without_substitution() {
        let (_fixture, mut runtime, handle) = setup("force={name=wind direction={1 0 0} amount=1} subsystem={name=s max_amount=1 duration=0 emission=4 life=4 force=wind}");
        let error = runtime.advance(1, 0.25, &[]).unwrap_err();
        assert_eq!(error.event_instance_id, Some(17));
        assert_eq!(error.subsystem.as_deref(), Some("s"));
        assert_eq!(error.input, "particle_motion");
        assert_eq!(error.reason, ParticleRefusalReason::ArithmeticOverflow);
        assert_eq!(runtime.light_binding(handle).unwrap_err(), error);
        assert_eq!(
            runtime.instances[handle.slot].as_ref().unwrap().subsystems[0].particles[0]
                .visual
                .columns[7],
            0.0
        );
    }

    #[test]
    fn force_free_motion_keeps_both_scale_factors_and_skips_zero_mass_division() {
        for (local, velocity, position) in [(false, 8.0, 11.0), (true, 4.0, 1.0)] {
            let (_fixture, mut runtime, handle) = setup(&format!("subsystem={{name=s max_amount=1 duration=0 emission=4 life=4 emitter_yaw=-90 velocity=4 local_space={}}}", if local { "yes" } else { "no" }));
            runtime.advance(1, 0.25, &[]).unwrap();
            let system = &runtime.instance(handle).unwrap().subsystems[0];
            assert_eq!(system.particles[0].visual.columns[7], 0.0);
            assert_eq!(system.particles[0].visual.velocity[0], velocity);
            assert_eq!(system.particles[0].visual.position[0], position);
            assert!(system.particles[0]
                .visual
                .position
                .iter()
                .all(|v| v.is_finite()));
        }
    }

    #[test]
    fn planar_and_friction_accumulate_before_mass_and_position_updates() {
        let (_fixture, mut runtime, handle) = setup("force={name=wind direction={1 0 0} amount=4} force={name=drag type=friction amount=.5} subsystem={name=s max_amount=1 duration=0 emission=4 life=4 emitter_yaw=-90 velocity=4 mass=2 local_space=yes force={wind drag}}");
        runtime.advance(1, 0.25, &[]).unwrap();
        let system = &runtime.instance(handle).unwrap().subsystems[0];
        assert_eq!(system.particles[0].visual.velocity, [4.25, 0.0, 0.0]);
        assert_eq!(system.particles[0].visual.position, [1.0625, 0.0, 0.0]);
        assert_eq!(system.force_scratch.len(), 1);
    }

    #[test]
    fn divided_forces_reuse_prefix_samples_by_modulo() {
        let (_fixture, mut runtime, handle) = setup("force={name=drag type=friction amount=1} subsystem={name=s max_amount=4 duration=0 emission=16 life=4 mass=1 local_space=yes force=drag slave_particles=1}");
        runtime.advance(1, 0.25, &[]).unwrap();
        let instance = runtime.instances[handle.slot].as_mut().unwrap();
        let system = &mut instance.subsystems[0];
        for (i, particle) in system.particles.iter_mut().enumerate() {
            particle.visual.velocity = [(i + 1) as f32 * 10.0, 0.0, 0.0];
        }
        system
            .advance_motion(
                &instance.definition.subsystems[0],
                0.1,
                1.0,
                &matrix().map(f32::from_bits),
                &mut instance.random,
            )
            .unwrap();
        assert_eq!(
            system
                .particles
                .iter()
                .map(|p| p.visual.velocity[0])
                .collect::<Vec<_>>(),
            [9.0, 18.0, 27.0, 39.0]
        );
        assert_eq!(system.force_scratch.len(), 4);
    }

    #[test]
    fn public_rotated_vortex_matches_native_writer_and_force_instruction_bits() {
        let fixture: serde_json::Value =
            serde_json::from_str(portable_engine_evidence::particle_runtime::FORCE_CASES).unwrap();
        let mut checked = 0;
        for case in fixture["cases"].as_array().unwrap() {
            if case["fma"].as_bool().unwrap()
                != portable_render_stream::exact_math::viewer_uses_fma()
            {
                continue;
            }
            let yaw = f32::from_bits(case["yaw_f32_bits"].as_u64().unwrap() as u32);
            let direction: [f32; 3] = std::array::from_fn(|i| {
                f32::from_bits(case["direction_f32_bits"][i].as_u64().unwrap() as u32)
            });
            let expected: [u32; 3] =
                std::array::from_fn(|i| case["force_f32_bits"][i].as_u64().unwrap() as u32);
            let (_fixture, mut runtime, handle) = setup(&format!(
                "force={{name=f type=vortex amount=1 position={{-2 0 0}} direction={{{} {} {}}} yaw={yaw}}} subsystem={{name=s max_amount=1 duration=0 emission=4 life=4 mass=1 force=f}}",
                direction[0], direction[1], direction[2],
            ));
            runtime.advance(1, 0.25, &[]).unwrap();
            let system = &runtime.instance(handle).unwrap().subsystems[0];
            assert_eq!(
                system.force_scratch[0].map(f32::to_bits),
                expected,
                "{case}"
            );
            assert!(system.particles[0]
                .visual
                .position
                .iter()
                .all(|v| v.is_finite()));
            assert_eq!(gpu_records(&runtime, handle).len(), 1);
            assert_eq!(
                runtime.light_binding(handle).unwrap().world_matrix_f32_bits,
                matrix()
            );
            checked += 1;
        }
        assert_eq!(checked, 27);
    }

    #[test]
    fn point_and_zero_yaw_vortex_use_strict_distance_and_local_translation() {
        for kind in ["point", "vortex"] {
            let (_fixture, mut runtime, handle) = setup(&format!("force={{name=f type={kind} amount=2 direction={{0 1 0}}}} subsystem={{name=s max_amount=1 duration=0 emission=4 life=4 mass=1 force=f}}"));
            runtime.advance(1, 0.25, &[]).unwrap();
            let instance = runtime.instances[handle.slot].as_mut().unwrap();
            let system = &mut instance.subsystems[0];
            let mut world = matrix().map(f32::from_bits);
            world[12] = 0.0;
            world[14] = 0.0;
            system.particles[0].visual.position = [f32::from_bits(0x3c23_d70a), 0.0, 0.0];
            system.particles[0].visual.velocity = [0.0; 3];
            system
                .advance_motion(
                    &instance.definition.subsystems[0],
                    0.5,
                    1.0,
                    &world,
                    &mut instance.random,
                )
                .unwrap();
            assert_eq!(system.particles[0].visual.velocity, [0.0; 3]);
            world[12] = 5.0;
            system.particles[0].visual.position = [7.0, 0.0, 0.0];
            system
                .advance_motion(
                    &instance.definition.subsystems[0],
                    0.5,
                    1.0,
                    &world,
                    &mut instance.random,
                )
                .unwrap();
            assert_eq!(system.particles[0].visual.velocity, [1.0, 0.0, 0.0]);
            assert_eq!(system.particles[0].visual.position, [7.5, 0.0, 0.0]);
        }
    }

    #[test]
    fn cpu_swap_keeps_gpu_source_and_swaps_cpu_birth_state() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=2 duration=0 emission=8 life=4 size={2 1}}");
        runtime.advance(1, 0.25, &[]).unwrap();
        let system = &mut runtime.instances[handle.slot].as_mut().unwrap().subsystems[0];
        system.particles[0].visual.position = [1.0, 2.0, 3.0];
        system.particles[1].visual.position = [4.0, 5.0, 6.0];
        system.particles[0].visual.velocity = [7.0, 8.0, 9.0];
        system.particles[1].visual.velocity = [10.0, 11.0, 12.0];
        let before = [system.particles[0], system.particles[1]];
        system.swap_cpu_copy_gpu(1, 0);
        assert_eq!(
            system.particles[0].visual.position,
            before[1].visual.position
        );
        assert_eq!(
            system.particles[1].visual.position,
            before[1].visual.position
        );
        assert_eq!(
            system.particles[0].visual.velocity,
            before[1].visual.velocity
        );
        assert_eq!(
            system.particles[1].visual.velocity,
            before[0].visual.velocity
        );
        assert_eq!(
            system.particles[1].visual.birth_columns,
            before[0].visual.birth_columns
        );
        assert_eq!(
            system.particles[1].visual.columns[..4],
            before[1].visual.columns[..4]
        );
        assert_eq!(
            (
                system.particles[0].child_slot,
                system.particles[1].child_slot
            ),
            (1, 0)
        );
    }

    #[test]
    fn visual_tracks_use_birth_samples_and_rotation_integrates_after_track_update() {
        let (_fixture, mut runtime, handle) = setup(
            "animation={name=grow time=life_abs minValue=8 maxValue=8 curve={0 0 1 1}}              subsystem={name=s max_amount=1 duration=0 emission=4 life=2              size=2,grow particle_roll=3 rotation_speed=4 color={r=300 g=-2 b=10 a=128}}",
        );
        runtime.advance(1, 0.25, &[]).unwrap();
        let first = runtime.instance(handle).unwrap().subsystems[0].particles[0];
        assert_eq!(first.visual.birth_columns[0], 2.0);
        assert_eq!(first.visual.columns[0], 10.0);
        assert_eq!(first.visual.columns[3], 4.0);
        assert_eq!(&first.visual.birth_columns[8..], &[10.0, 0.0, 255.0, 128.0]);
        let random = runtime.instance(handle).unwrap().random;
        runtime.advance(2, 0.25, &[]).unwrap();
        let second = runtime.instance(handle).unwrap().subsystems[0].particles[0];
        assert_eq!(second.visual.columns[0], 10.0);
        assert_eq!(second.visual.columns[3], 5.0);
        assert_eq!(second.visual.atlas_draws, first.visual.atlas_draws);
        assert_eq!(runtime.instance(handle).unwrap().random, random);
        assert_eq!(second.age, 0.25);
    }

    #[test]
    fn retained_visual_samples_follow_particle_compaction_without_resampling() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=3 duration=0 emission=12 life=2 size={2 1}}");
        runtime.advance(1, 0.25, &[]).unwrap();
        let instance = runtime.instances[handle.slot].as_mut().unwrap();
        let survivor = instance.subsystems[0].particles[2];
        instance.subsystems[0].particles[0].life = 0.0;
        let random = instance.random;
        runtime.advance(2, 0.25, &[]).unwrap();
        let instance = runtime.instance(handle).unwrap();
        let subsystem = &instance.subsystems[0];
        assert_eq!(subsystem.live, 2);
        assert_eq!(subsystem.particles[0].child_slot, survivor.child_slot);
        assert_eq!(
            subsystem.particles[0].visual.birth_columns,
            survivor.visual.birth_columns
        );
        assert_eq!(
            subsystem.particles[0].visual.atlas_draws,
            survivor.visual.atlas_draws
        );
        assert_eq!(instance.random, random);
    }

    #[test]
    fn trail_death_preserves_order_and_duplicates_the_final_child_identity() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=4 duration=0 emission=16 life=2 trail=yes}");
        runtime.advance(1, 0.25, &[]).unwrap();
        let instance = runtime.instances[handle.slot].as_mut().unwrap();
        instance.subsystems[0].particles[1].life = 0.0;
        let random = instance.random;
        runtime.advance(2, 0.25, &[]).unwrap();
        let instance = runtime.instance(handle).unwrap();
        let system = &instance.subsystems[0];
        assert_eq!((system.live, system.retained), (3, 3));
        assert_eq!(
            system
                .particles
                .iter()
                .map(|p| p.child_slot)
                .collect::<Vec<_>>(),
            [0, 2, 3, 3]
        );
        assert_eq!(instance.random, random);
    }

    #[test]
    fn trail_overlapping_retained_parent_skips_birth_writers_but_advances_counts() {
        for amount in [1, 3] {
            let (_fixture, mut runtime, handle) = setup(
                "subsystem={name=s max_amount=4 duration=-1 emission=4 life={2 1} trail=yes childsystem={name=c max_amount=1 duration=-1 emission=0 life=8}}");
            runtime.advance(1, 0.25, &[]).unwrap();
            let instance = runtime.instances[handle.slot].as_mut().unwrap();
            let system = &mut instance.subsystems[0];
            system.live = 0;
            system.fractional_emission = (amount - 1) as f32;
            system.children[0].live = 1;
            system.children[0].retained = 1;
            system.children[0].particles[0].life = 8.0;
            let prior = system.particles[0];
            let random = instance.random;
            runtime.advance(2, 0.25, &[]).unwrap();
            let instance = runtime.instance(handle).unwrap();
            let system = &instance.subsystems[0];
            assert_eq!((system.live, system.retained), (amount, amount + 1));
            assert_eq!(system.fractional_emission.to_bits(), 0.0_f32.to_bits());
            assert_eq!(system.particles[0].life.to_bits(), prior.life.to_bits());
            assert_eq!(
                system.particles[0].visual.atlas_draws,
                prior.visual.atlas_draws
            );
            assert_eq!(system.children[0].retained, 1);
            assert_eq!(instance.random, random);
            let records = gpu_records(&runtime, handle);
            assert_eq!(records.len(), amount);
            for record in &records[1..] {
                assert_eq!(record.0, [0; 48]);
            }
            let children: Vec<_> = runtime
                .gpu_view(handle)
                .unwrap()
                .root_subsystems()
                .next()
                .unwrap()
                .children()
                .map(|(parent, child, view)| (parent, child, view.live_count()))
                .collect();
            assert_eq!(children.len(), amount + 1);
            assert_eq!(children[0], (0, 0, 1));
            for slot in &system.particles[1..amount] {
                assert_eq!([slot.age.to_bits(), slot.life.to_bits()], [0; 2]);
                assert_eq!(slot.visual.birth_columns.map(f32::to_bits), [0; 12]);
                assert_eq!(slot.visual.columns.map(f32::to_bits), [0; 12]);
                assert_eq!(slot.visual.atlas_draws, [0; 2]);
                assert_eq!(slot.visual.emitter_angles.map(f32::to_bits), [0; 2]);
                assert_eq!(
                    slot.visual.velocity_angles_and_speed.map(f32::to_bits),
                    [0; 3]
                );
                assert_eq!(slot.visual.shape_coordinates.map(f32::to_bits), [0; 3]);
            }
        }
    }

    #[test]
    fn conditional_state_stop_preserves_finite_ends_and_stops_every_root_if_one_is_unbounded() {
        for (end, expected) in [(2.0, [1.0, 2.0]), (-1.0, [0.25, 0.25])] {
            let (_fixture, mut runtime, handle) = setup(&format!(
                "subsystem={{name=a max_amount=0 duration=1 emission=0}} subsystem={{name=b max_amount=0 duration={end} emission=0}}"));
            runtime.advance(1, 0.25, &[]).unwrap();
            runtime
                .advance(
                    2,
                    0.25,
                    &[ParticleInstanceUpdate {
                        handle,
                        parent_world_matrix_f32_bits: None,
                        inherited_speed: None,
                        action: ParticleAction::StopUnboundedEmission,
                    }],
                )
                .unwrap();
            let subsystems = &runtime.instance(handle).unwrap().subsystems;
            assert_eq!([subsystems[0].end, subsystems[1].end], expected);
        }
    }

    #[test]
    fn finite_end_waits_for_strict_particle_death_boundary() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=a max_amount=4 start=0 duration=0 emission=4 life=0.5}");
        assert!(!runtime.light_binding(handle).unwrap().finished);
        runtime.advance(1, 0.25, &[]).unwrap();
        assert!(!runtime.light_binding(handle).unwrap().finished);
        runtime.advance(2, 0.25, &[]).unwrap();
        runtime.advance(3, 0.25, &[]).unwrap();
        assert_eq!(
            runtime.instance(handle).unwrap().subsystems[0].particles[0].age,
            0.5
        );
        assert!(!runtime.light_binding(handle).unwrap().finished);
        runtime.advance(4, 0.25, &[]).unwrap();
        assert!(runtime.light_binding(handle).unwrap().finished);
    }

    #[test]
    fn negative_end_and_empty_roster_have_different_finished_truth() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=a max_amount=0 duration=-1 emission=0}");
        runtime.advance(1, 9.0, &[]).unwrap();
        assert!(!runtime.light_binding(handle).unwrap().finished);
        let (_fixture, runtime, handle) = setup("");
        assert!(runtime.light_binding(handle).unwrap().finished);
    }

    #[test]
    fn stop_preserves_particles_and_the_engine_equality_emission_step() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=a max_amount=8 duration=-1 emission=4 life=0.5}");
        runtime.advance(1, 0.25, &[]).unwrap();
        runtime
            .advance(
                2,
                0.25,
                &[ParticleInstanceUpdate {
                    handle,
                    parent_world_matrix_f32_bits: None,
                    inherited_speed: None,
                    action: ParticleAction::StopEmission,
                }],
            )
            .unwrap();
        assert_eq!(runtime.instance(handle).unwrap().subsystems[0].live, 2);
        assert!(!runtime.light_binding(handle).unwrap().finished);
        runtime.advance(3, 0.25, &[]).unwrap();
        runtime.advance(4, 0.25, &[]).unwrap();
        assert!(!runtime.light_binding(handle).unwrap().finished);
        runtime.advance(5, 0.25, &[]).unwrap();
        assert!(runtime.light_binding(handle).unwrap().finished);
    }

    #[test]
    fn pulse_uses_strict_boundaries_and_resets_time_before_last_increment() {
        let (_fixture, mut runtime, handle) = setup("subsystem={name=a max_amount=8 duration=-1 emission=4 life=8 emission_pulse_duration=0.25 emission_pulse_silence=0.25}");
        runtime.advance(1, 0.25, &[]).unwrap();
        runtime.advance(2, 0.25, &[]).unwrap();
        runtime.advance(3, 0.25, &[]).unwrap();
        let state = &runtime.instance(handle).unwrap().subsystems[0];
        assert_eq!(state.live, 1);
        assert!(!state.pulse_on);
        runtime.advance(4, 0.25, &[]).unwrap();
        let state = &runtime.instance(handle).unwrap().subsystems[0];
        assert_eq!(state.live, 2);
        assert_eq!(state.local_time, 0.5);
        assert!(state.pulse_on);
    }

    #[test]
    fn exact_matrix_bits_survive_scale_attachment_and_detachment() {
        let (_fixture, mut runtime, handle) = setup("");
        assert_eq!(
            runtime.light_binding(handle).unwrap().world_matrix_f32_bits,
            matrix()
        );
        let mut moved = matrix();
        moved[12] = 99.0_f32.to_bits();
        runtime.refresh_attachment(handle, moved, 2.0).unwrap();
        assert_eq!(
            runtime.light_binding(handle).unwrap().world_matrix_f32_bits,
            moved
        );
        runtime
            .advance(
                1,
                0.0,
                &[ParticleInstanceUpdate {
                    handle,
                    parent_world_matrix_f32_bits: None,
                    inherited_speed: None,
                    action: ParticleAction::Detach,
                }],
            )
            .unwrap();
        runtime.refresh_attachment(handle, matrix(), 1.0).unwrap();
        assert_eq!(
            runtime.light_binding(handle).unwrap().world_matrix_f32_bits,
            moved
        );
    }

    #[test]
    fn seeded_life_columns_and_atlas_draws_are_repeatable_and_bounded() {
        let body = "subsystem={name=a max_amount=4 duration=-1 emission=2 life={0.5 0.25}}";
        let (_fixture_a, mut a, ha) = setup(body);
        let (_fixture_b, mut b, hb) = setup(body);
        let allocated = a.allocated_particle_capacity();
        for frame in 0..20_000 {
            a.advance(frame, 0.25, &[]).unwrap();
            b.advance(frame, 0.25, &[]).unwrap();
            let left = a.instance(ha).unwrap();
            let right = b.instance(hb).unwrap();
            assert_eq!(left.random, right.random);
            assert!(left.subsystems[0].live <= 4);
            assert_eq!(a.allocated_particle_capacity(), allocated);
            let l: Vec<_> = left.subsystems[0].particles[..left.subsystems[0].retained]
                .iter()
                .map(|p| (p.age.to_bits(), p.life.to_bits()))
                .collect();
            let r: Vec<_> = right.subsystems[0].particles[..right.subsystems[0].retained]
                .iter()
                .map(|p| (p.age.to_bits(), p.life.to_bits()))
                .collect();
            assert_eq!(l, r);
        }
        assert_eq!(a.instance(ha).unwrap().random.counter, 30_000);
    }

    #[test]
    fn ranged_columns_consume_in_engine_order_including_unused_shape_and_atlas() {
        let (_fixture, mut runtime, handle) = setup("subsystem={name=a max_amount=4 duration=0 emission=4 life={1 0.25} emitter_type=sphere sphere_emitter_radius={1 1} sphere_emitter_yaw={0 180} sphere_emitter_pitch={0 90} emitter_yaw={0 180} emitter_pitch={0 90} velocity={1 0.5}}");
        runtime.advance(1, 0.5, &[]).unwrap();
        assert_eq!(runtime.instance(handle).unwrap().random.counter, 16);
        assert_eq!(runtime.instance(handle).unwrap().subsystems[0].live, 2);
    }

    #[test]
    fn duplicate_frame_cannot_double_step_and_seek_invalidates_old_handles() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=a max_amount=4 duration=-1 emission=4 life=1}");
        runtime.advance(10, 0.25, &[]).unwrap();
        let before = runtime.instance(handle).unwrap().random;
        assert_eq!(
            runtime.advance(10, 0.25, &[]).unwrap_err().reason,
            ParticleRefusalReason::DuplicatePresentation
        );
        assert_eq!(
            runtime.advance(9, 0.25, &[]).unwrap_err().reason,
            ParticleRefusalReason::PresentationOutOfOrder
        );
        assert_eq!(runtime.instance(handle).unwrap().random, before);
        runtime.reset();
        assert_eq!(
            runtime.light_binding(handle).unwrap_err().reason,
            ParticleRefusalReason::UnknownHandle
        );
        assert_eq!(runtime.live_instance_count(), 0);
    }

    #[test]
    fn foreign_definition_handle_and_duplicate_event_cannot_alias() {
        let (fixture, mut a, ha) = setup("");
        let mut definitions = ParticleDefinitions::load(&fixture.resolver).unwrap();
        let mut b = ParticleRuntime::new(definitions.content_lease(), 1).unwrap();
        assert_eq!(
            b.light_binding(ha).unwrap_err().reason,
            ParticleRefusalReason::UnknownHandle
        );
        let definition = definitions.resolve("paired").unwrap();
        assert_eq!(
            a.spawn(
                17,
                Arc::clone(&definition),
                ViewerParticleSeed {
                    counter: 0,
                    seed: 0
                },
                matrix(),
                1.0
            )
            .unwrap_err()
            .reason,
            ParticleRefusalReason::DuplicateEventInstance
        );
        let other = Fixture::new(REGISTRATION, "particle={name=effect}");
        let foreign = other.definitions().resolve("paired").unwrap();
        assert_eq!(
            b.spawn(
                2,
                foreign,
                ViewerParticleSeed {
                    counter: 0,
                    seed: 0
                },
                matrix(),
                1.0
            )
            .unwrap_err()
            .reason,
            ParticleRefusalReason::WrongContentSession
        );
        b.spawn(
            2,
            definition,
            ViewerParticleSeed {
                counter: 0,
                seed: 0,
            },
            matrix(),
            1.0,
        )
        .unwrap();
    }

    #[test]
    fn destruction_releases_slot_and_stale_handle_never_rebinds() {
        let (fixture, mut runtime, handle) = setup("");
        runtime
            .advance(
                1,
                0.0,
                &[ParticleInstanceUpdate {
                    handle,
                    parent_world_matrix_f32_bits: None,
                    inherited_speed: None,
                    action: ParticleAction::Destroy,
                }],
            )
            .unwrap();
        let definition = fixture.definitions().resolve("paired").unwrap();
        let replacement = runtime
            .spawn(
                17,
                definition,
                ViewerParticleSeed {
                    counter: 0,
                    seed: 29,
                },
                matrix(),
                1.0,
            )
            .unwrap();
        assert_ne!(handle, replacement);
        assert!(runtime.light_binding(handle).is_err());
        assert!(runtime.light_binding(replacement).is_ok());
    }
    #[test]
    fn dead_parent_retains_children_until_the_next_cleanup_phase() {
        let (_fixture, mut runtime, handle) = setup("subsystem={name=root max_amount=2 duration=0 emission=4 life=0.25 childsystem={name=trail max_amount=4 duration=-1 emission=4 life=0.5}}");
        assert_eq!(runtime.allocated_particle_capacity(), 10);
        for frame in 1..=5 {
            runtime.advance(frame, 0.25, &[]).unwrap();
            assert!(!runtime.light_binding(handle).unwrap().finished);
            assert_eq!(runtime.instance(handle).unwrap().subsystems[0].retained, 1);
            if frame >= 3 {
                assert_eq!(runtime.instance(handle).unwrap().subsystems[0].live, 0);
            }
        }
        runtime.advance(6, 0.25, &[]).unwrap();
        assert!(runtime.light_binding(handle).unwrap().finished);
        assert_eq!(runtime.allocated_particle_capacity(), 10);
    }

    #[test]
    fn every_child_capacity_slot_is_initialized_before_the_parent() {
        let (_fixture, runtime, handle) = setup("subsystem={name=root max_amount=3 duration={1 0.1} childsystem={name=trail max_amount=2 duration={1 0.1} start={1 0.1}}}");
        let state = runtime.instance(handle).unwrap();
        assert_eq!(state.random.counter, 7);
        let mut random = ViewerParticleSeed {
            counter: 0,
            seed: 29,
        };
        for child in &state.subsystems[0].children {
            assert_eq!(
                child.end.to_bits(),
                random.sample(&VariedScalar::plain(1.0, 0.1), 0.0).to_bits()
            );
            assert_eq!(
                child.local_time.to_bits(),
                random.sample(&VariedScalar::plain(1.0, 0.1), 0.0).to_bits() ^ 0x8000_0000
            );
        }
        assert_eq!(
            state.subsystems[0].end.to_bits(),
            random.sample(&VariedScalar::plain(1.0, 0.1), 0.0).to_bits()
        );
    }

    #[test]
    fn retained_child_identity_moves_beyond_the_entire_new_birth_batch() {
        let (_fixture, mut runtime, handle) = setup("subsystem={name=root max_amount=4 duration=-1 emission=4 life=1 childsystem={name=trail max_amount=1 duration=-1 emission=0 life=10}}");
        let root = &mut runtime.instances[handle.slot].as_mut().unwrap().subsystems[0];
        root.retained = 1;
        root.children[0].live = 1;
        root.children[0].retained = 1;
        root.children[0].particles[0].life = 10.0;
        runtime.advance(1, 0.5, &[]).unwrap();
        let root = &runtime.instance(handle).unwrap().subsystems[0];
        assert_eq!((root.live, root.retained), (2, 3));
        assert_eq!(
            root.particles[..3]
                .iter()
                .map(|p| p.child_slot)
                .collect::<Vec<_>>(),
            [2, 1, 0]
        );
        assert_eq!(root.children[0].retained, 1);
        assert_eq!(root.children[0].particles[0].age, 0.5);
    }

    #[test]
    fn full_capacity_preserves_fractional_backlog_for_the_next_vacancy() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=1 duration=-1 emission=4 life=0.5}");
        runtime.advance(1, 0.25, &[]).unwrap();
        runtime.advance(2, 0.25, &[]).unwrap();
        runtime.advance(3, 0.25, &[]).unwrap();
        assert_eq!(
            runtime.instance(handle).unwrap().subsystems[0].fractional_emission,
            2.0
        );
        runtime.advance(4, 0.25, &[]).unwrap();
        let state = &runtime.instance(handle).unwrap().subsystems[0];
        assert_eq!(state.live, 1);
        assert_eq!(state.particles[0].age, 0.0);
        assert_eq!(state.fractional_emission, 2.0);
    }

    #[test]
    fn duplicate_notifications_are_rejected_before_attachment_or_clock_mutation() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=1 duration=-1 emission=1 life=1}");
        let update = ParticleInstanceUpdate {
            handle,
            parent_world_matrix_f32_bits: Some([0; 16]),
            inherited_speed: Some(2.0),
            action: ParticleAction::Continue,
        };
        assert_eq!(
            runtime
                .advance(1, 0.25, &[update, update])
                .unwrap_err()
                .reason,
            ParticleRefusalReason::DuplicateNotification
        );
        let binding = runtime.light_binding(handle).unwrap();
        assert_eq!(binding.presentation_id, None);
        assert_eq!(binding.world_matrix_f32_bits, matrix());
        assert_eq!(runtime.instance(handle).unwrap().random.counter, 0);
        runtime.advance(1, 0.25, &[update]).unwrap();
        assert_eq!(
            runtime.instance(handle).unwrap().subsystems[0].local_time,
            0.5
        );
    }

    #[test]
    fn once_range_curve_selects_random_callback_even_for_zero_output() {
        let (_fixture, mut runtime, handle) = setup("animation={name=zero curve={0 0 1 1}} subsystem={name=s max_amount=1 duration=0 emission=4 life={1 zero}}");
        runtime.advance(1, 0.25, &[]).unwrap();
        let state = runtime.instance(handle).unwrap();
        assert_eq!(state.random.counter, 3);
        assert_eq!(state.subsystems[0].particles[0].life, 1.0);
    }

    #[test]
    fn turbulence_advances_the_instance_seed_before_first_particle_life() {
        let (_fixture, mut runtime, handle) = setup("force={name=noise type=turbulence} subsystem={name=s max_amount=1 duration=0 emission=4 life={1 0.5}}");
        let before = runtime.instance(handle).unwrap().random;
        assert!(before.counter > 2047);
        let mut next = before;
        let expected = next.sample(&VariedScalar::plain(1.0, 0.5), 0.0);
        runtime.advance(1, 0.25, &[]).unwrap();
        let state = runtime.instance(handle).unwrap();
        assert_eq!(
            state.subsystems[0].particles[0].life.to_bits(),
            expected.to_bits()
        );
        assert_eq!(state.random.counter, before.counter + 3);
    }

    #[test]
    fn local_space_changes_neither_instance_matrix_nor_light_liveness() {
        let (_a, mut world, hw) =
            setup("subsystem={name=s max_amount=1 duration=0 emission=4 life=0.5 local_space=no}");
        let (_b, mut local, hl) =
            setup("subsystem={name=s max_amount=1 duration=0 emission=4 life=0.5 local_space=yes}");
        for frame in 1..=4 {
            world.advance(frame, 0.25, &[]).unwrap();
            local.advance(frame, 0.25, &[]).unwrap();
            let world = world.light_binding(hw).unwrap();
            let local = local.light_binding(hl).unwrap();
            assert_eq!(world.world_matrix_f32_bits, local.world_matrix_f32_bits);
            assert_eq!(world.finished, local.finished);
        }
    }

    #[test]
    fn combined_stop_and_detach_keeps_the_boundary_birth_and_last_parent_matrix() {
        let (_fixture, mut runtime, handle) =
            setup("subsystem={name=s max_amount=4 duration=-1 emission=4 life=0.5}");
        let mut moved = matrix();
        moved[12] = 91.0_f32.to_bits();
        runtime
            .advance(
                1,
                0.25,
                &[ParticleInstanceUpdate {
                    handle,
                    parent_world_matrix_f32_bits: Some(moved),
                    inherited_speed: None,
                    action: ParticleAction::StopAndDetach,
                }],
            )
            .unwrap();
        runtime.refresh_attachment(handle, matrix(), 1.0).unwrap();
        assert_eq!(
            runtime.light_binding(handle).unwrap().world_matrix_f32_bits,
            moved
        );
        assert_eq!(runtime.instance(handle).unwrap().subsystems[0].live, 1);
        assert!(!runtime.light_binding(handle).unwrap().finished);
        for frame in 2..=4 {
            runtime.advance(frame, 0.25, &[]).unwrap();
        }
        assert!(runtime.light_binding(handle).unwrap().finished);
    }
}
