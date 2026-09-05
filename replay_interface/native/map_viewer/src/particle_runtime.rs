use crate::mod_resolver::ContentAdmissionLease;
use crate::particle_authored::{
    AdmittedParticleDefinition, AuthoredSubsystem, ParticleInputRefusal, ParticleRefusalReason,
    ParticleResult, VariedScalar,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleLightBinding {
    pub event_instance_id: u64,
    pub presentation_id: Option<u64>,
    pub world_matrix_f32_bits: [u32; 16],
    pub finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleAction {
    Continue,
    StopEmission,
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

#[derive(Clone, Copy, Debug)]
struct LiveParticle {
    age: f32,
    life: f32,
    child_slot: usize,
}

#[derive(Debug)]
struct Subsystem {
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
            });
            for child in &authored.children {
                children.push(Self::new(child, random)?);
            }
        }
        for origin in &authored.origin {
            random.sample(origin, 0.0);
        }
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
        Ok(Self {
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

    fn advance(
        &mut self,
        authored: &AuthoredSubsystem,
        delta: f32,
        random: &mut ViewerParticleSeed,
        parent_alive: bool,
    ) -> ParticleResult<()> {
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
                self.particles.swap(index, self.live);
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
                self.particles.swap(index, self.retained);
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
            for index in 0..amount.min(self.retained - self.live) {
                self.particles.swap(
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
                for property in &authored.spawn_draw_columns {
                    for _ in 0..amount {
                        random.sample(property, self.local_time);
                    }
                }
                for property in &authored.emitter_angles {
                    random.sample(property, self.local_time);
                }
                for property in &authored.velocity_angles_and_speed {
                    for _ in 0..amount {
                        random.sample(property, self.local_time);
                    }
                }
                if let Some(properties) = &authored.shape {
                    for property in properties {
                        for _ in 0..amount {
                            random.sample(property, self.local_time);
                        }
                    }
                }
            }
            for _ in 0..amount {
                random.draw();
                random.draw();
            }
            self.live += amount;
            self.retained += amount;
            self.fractional_emission -= amount as f32;
        }
        for index in 0..self.retained {
            let first = self.particles[index].child_slot * child_count;
            for (child, definition) in self.children[first..first + child_count]
                .iter_mut()
                .zip(&authored.children)
            {
                child.advance(definition, delta, random, index < self.live)?;
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
    attached: bool,
    subsystems: Vec<Subsystem>,
    failure: Option<ParticleInputRefusal>,
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
            attached: true,
            subsystems,
            failure: None,
        });
        Ok(handle)
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
            if failure.is_none() {
                for (state, authored) in instance
                    .subsystems
                    .iter_mut()
                    .zip(instance.definition.subsystems.iter())
                {
                    if let Err(error) = state.advance(authored, delta, &mut instance.random, true) {
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
